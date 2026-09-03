//! Join handles and the structured combinators: `join_all`, `join_all_ok`,
//! `select2`, `select_n`, `map_join`.
//!
//! Results travel through a small loom-friendly one-shot slot
//! (`Arc<Mutex<Option<..>>>`) instead of `std::mpsc` (whose `Sender` is `!Sync`
//! and not loom-instrumented). A `SlotSetter` fills `Err(Panicked)` on drop,
//! preserving the "producer vanished ⇒ consumer sees an error" semantics the
//! old mpsc channel got from disconnection.

use crate::ids::TaskId;
use crate::sync::{Arc, Mutex};
use crate::task::{Cx, JoinError, JoinPoll, ScopedTask, TaskPoll};

pub(crate) type ResultSlot<T> = Arc<Mutex<Option<Result<T, JoinError>>>>;

/// Write end of a one-shot result. Fills `Err(Panicked)` if dropped unfilled.
pub(crate) struct SlotSetter<T> {
    slot: ResultSlot<T>,
    sent: bool,
}

impl<T> SlotSetter<T> {
    pub(crate) fn send(&mut self, v: Result<T, JoinError>) {
        if !self.sent {
            *self.slot.lock().unwrap() = Some(v);
            self.sent = true;
        }
    }
}

impl<T> Drop for SlotSetter<T> {
    fn drop(&mut self) {
        if !self.sent {
            *self.slot.lock().unwrap() = Some(Err(JoinError::Panicked));
        }
    }
}

pub(crate) fn result_channel<T>() -> (SlotSetter<T>, ResultSlot<T>) {
    let slot: ResultSlot<T> = Arc::new(Mutex::new(None));
    (SlotSetter { slot: slot.clone(), sent: false }, slot)
}

/// Handle to a spawned task's eventual `Result<T, JoinError>`.
pub struct JoinHandle<T> {
    /// The task whose result this handle waits for.
    pub task: TaskId,
    slot: ResultSlot<T>,
}

impl<T> JoinHandle<T> {
    pub(crate) fn new(task: TaskId, slot: ResultSlot<T>) -> Self {
        Self { task, slot }
    }

    /// Non-consuming poll. `Ready` once, then `Pending` (the value is taken).
    pub fn poll(&self) -> JoinPoll<T> {
        match self.slot.lock().unwrap().take() {
            Some(r) => JoinPoll::Ready(r),
            None => JoinPoll::Pending,
        }
    }

    /// Take the result if present. Useful after `Scope::run` has completed.
    pub fn take(&self) -> Option<Result<T, JoinError>> {
        self.slot.lock().unwrap().take()
    }
}

pub use crate::select::Either;

// --------------------------------------------------------------------------
// select2
// --------------------------------------------------------------------------

pub(crate) struct Select2Task<A, B> {
    pub(crate) prod_a: TaskId,
    pub(crate) prod_b: TaskId,
    pub(crate) a: Option<JoinHandle<A>>,
    pub(crate) b: Option<JoinHandle<B>>,
    pub(crate) sub_a: bool,
    pub(crate) sub_b: bool,
    pub(crate) flip: bool,
    pub(crate) out: SlotSetter<Either<Result<A, JoinError>, Result<B, JoinError>>>,
}

impl<'env, R, A, B> ScopedTask<'env, R> for Select2Task<A, B>
where
    A: 'env,
    B: 'env,
{
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        for _ in 0..2 {
            if !self.flip {
                if let Some(a) = self.a.as_ref() {
                    match a.poll() {
                        JoinPoll::Pending => {
                            if !self.sub_a {
                                self.sub_a = true;
                                cx.subscribe_to_finish(self.prod_a);
                            }
                        }
                        JoinPoll::Ready(res) => {
                            self.out.send(Ok(Either::Left(res)));
                            self.a = None;
                            self.b = None;
                            return TaskPoll::Ready;
                        }
                    }
                }
            } else if let Some(b) = self.b.as_ref() {
                match b.poll() {
                    JoinPoll::Pending => {
                        if !self.sub_b {
                            self.sub_b = true;
                            cx.subscribe_to_finish(self.prod_b);
                        }
                    }
                    JoinPoll::Ready(res) => {
                        self.out.send(Ok(Either::Right(res)));
                        self.a = None;
                        self.b = None;
                        return TaskPoll::Ready;
                    }
                }
            }
            self.flip = !self.flip;
        }
        TaskPoll::Pending
    }
}

// --------------------------------------------------------------------------
// select_n
// --------------------------------------------------------------------------

pub(crate) struct SelectNTask<T> {
    pub(crate) producers: Vec<TaskId>,
    pub(crate) joins: Vec<JoinHandle<T>>,
    pub(crate) subscribed: Vec<bool>,
    pub(crate) rr: usize,
    pub(crate) out: SlotSetter<(usize, Result<T, JoinError>)>,
}

impl<'env, R, T> ScopedTask<'env, R> for SelectNTask<T>
where
    T: 'env,
{
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        let n = self.joins.len();
        if n == 0 {
            self.out.send(Err(JoinError::Panicked));
            return TaskPoll::Ready;
        }
        for k in 0..n {
            let i = (self.rr + k) % n;
            match self.joins[i].poll() {
                JoinPoll::Pending => {
                    if !self.subscribed[i] {
                        self.subscribed[i] = true;
                        cx.subscribe_to_finish(self.producers[i]);
                    }
                }
                JoinPoll::Ready(res) => {
                    self.out.send(Ok((i, res)));
                    return TaskPoll::Ready;
                }
            }
        }
        self.rr = (self.rr + 1) % n;
        TaskPoll::Pending
    }
}

// --------------------------------------------------------------------------
// join_all
// --------------------------------------------------------------------------

pub(crate) struct JoinAllTask<T> {
    pub(crate) producers: Vec<TaskId>,
    pub(crate) joins: Vec<JoinHandle<T>>,
    pub(crate) results: Vec<Option<Result<T, JoinError>>>,
    pub(crate) subscribed: Vec<bool>,
    pub(crate) out: SlotSetter<Vec<Result<T, JoinError>>>,
}

impl<'env, R, T> ScopedTask<'env, R> for JoinAllTask<T>
where
    T: 'env,
{
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        for i in 0..self.joins.len() {
            if self.results[i].is_some() {
                continue;
            }
            match self.joins[i].poll() {
                JoinPoll::Pending => {
                    if !self.subscribed[i] {
                        self.subscribed[i] = true;
                        cx.subscribe_to_finish(self.producers[i]);
                    }
                }
                JoinPoll::Ready(res) => self.results[i] = Some(res),
            }
        }
        if self.results.iter().all(|r| r.is_some()) {
            let out = self.results.iter_mut().map(|r| r.take().unwrap()).collect::<Vec<_>>();
            self.out.send(Ok(out));
            return TaskPoll::Ready;
        }
        TaskPoll::Pending
    }
}

// --------------------------------------------------------------------------
// join_all_ok — stops on first Err(E) / panic, short-circuits the scope.
// --------------------------------------------------------------------------

#[derive(Debug)]
/// Why a `join_all_ok` produced no vector: the first failure, by position.
pub enum JoinAllOkError<E> {
    /// The task at `index` panicked.
    Panicked {
        /// Position of the task in the joined list.
        index: usize,
    },
    /// The task at `index` returned `Err(err)`.
    Err {
        /// Position of the task in the joined list.
        index: usize,
        /// The error it returned.
        err: E,
    },
    /// A join failed before a value existed: the task was cancelled or panicked.
    JoinFailed(JoinError),
}

pub(crate) struct JoinAllOkTask<T, E> {
    pub(crate) producers: Vec<TaskId>,
    pub(crate) joins: Vec<JoinHandle<Result<T, E>>>,
    pub(crate) subscribed: Vec<bool>,
    pub(crate) vals: Vec<Option<T>>,
    pub(crate) out: SlotSetter<Result<Vec<T>, JoinAllOkError<E>>>,
}

impl<'env, R, T, E> ScopedTask<'env, R> for JoinAllOkTask<T, E>
where
    T: 'env,
    E: 'env,
{
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        for i in 0..self.joins.len() {
            if self.vals[i].is_some() {
                continue;
            }
            match self.joins[i].poll() {
                JoinPoll::Pending => {
                    if !self.subscribed[i] {
                        self.subscribed[i] = true;
                        cx.subscribe_to_finish(self.producers[i]);
                    }
                }
                JoinPoll::Ready(res) => match res {
                    Err(JoinError::Panicked) => {
                        self.out.send(Ok(Err(JoinAllOkError::Panicked { index: i })));
                        return TaskPoll::Ready;
                    }
                    Err(JoinError::Cancelled(r)) => {
                        self.out.send(Err(JoinError::Cancelled(r)));
                        return TaskPoll::Ready;
                    }
                    Ok(Ok(v)) => self.vals[i] = Some(v),
                    Ok(Err(e)) => {
                        self.out.send(Ok(Err(JoinAllOkError::Err { index: i, err: e })));
                        return TaskPoll::Ready;
                    }
                },
            }
        }
        if self.vals.iter().all(|v| v.is_some()) {
            let out = self.vals.iter_mut().map(|v| v.take().unwrap()).collect::<Vec<_>>();
            self.out.send(Ok(Ok(out)));
            return TaskPoll::Ready;
        }
        TaskPoll::Pending
    }
}

// --------------------------------------------------------------------------
// map_join — the SAFE replacement for the original's UB `map_ok` transmute.
// --------------------------------------------------------------------------

pub(crate) struct MapJoinTask<T, Out, F> {
    pub(crate) producer: TaskId,
    pub(crate) join: Option<JoinHandle<T>>,
    pub(crate) f: Option<F>,
    pub(crate) subscribed: bool,
    pub(crate) out: SlotSetter<Out>,
}

impl<'env, R, T, Out, F> ScopedTask<'env, R> for MapJoinTask<T, Out, F>
where
    T: 'env,
    Out: 'env,
    F: for<'p> FnOnce(Result<T, JoinError>, &mut Cx<'p, 'env, R>) -> Out + 'env,
{
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        let Some(join) = self.join.as_ref() else { return TaskPoll::Ready };
        match join.poll() {
            JoinPoll::Pending => {
                if !self.subscribed {
                    self.subscribed = true;
                    cx.subscribe_to_finish(self.producer);
                }
                TaskPoll::Pending
            }
            JoinPoll::Ready(res) => {
                let f = self.f.take().expect("map-join f already taken");
                let mapped = f(res, cx);
                self.out.send(Ok(mapped));
                self.join = None;
                TaskPoll::Ready
            }
        }
    }
}
