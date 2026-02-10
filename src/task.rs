// src/task.rs
use std::{
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use crate::{
    wake::WakeTx,
    CancelEscalation,
    CancelReason,
};

pub type TaskId = usize;
pub const EXTERNAL_WAKE: TaskId = usize::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskPoll {
    Pending,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Alive,
    Completed,
    Cancelled,
    Panicked,
}

#[derive(Debug, Clone, Copy)]
pub struct WakeMsg {
    pub from: TaskId,
    pub to: TaskId,
}

#[derive(Clone)]
pub struct WakeSender {
    from: TaskId,
    tx: WakeTx,
}
impl WakeSender {
    #[inline]
    pub fn wake(&self, to: TaskId) {
        self.tx.send(WakeMsg { from: self.from, to });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinError {
    Panicked,
    Cancelled(CancelReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinPoll<T> {
    Pending,
    Ready(Result<T, JoinError>),
}

/// Tasks live inside a Scope and may borrow `R` via `'env`.
pub trait ScopedTask<'env, R>: 'env {
    fn poll(self: Pin<&mut Self>, cx: &mut Cx<'_, 'env, R>) -> TaskPoll;
}

/// The poll context for tasks.
pub struct Cx<'poll, 'env, R> {
    pub resources: &'poll mut R,

    pub(crate)id: TaskId,
    pub(crate)wake_tx: WakeTx,

    // Cancellation flag for *this* task.
    pub(crate)cancel_flag: Arc<AtomicBool>,

    // Hooks into Scope:
    pub(crate)subscribe_finish: &'poll mut dyn FnMut(TaskId, TaskId),
    pub(crate)defer_spawn: &'poll mut dyn FnMut(TaskId, String, Pin<Box<dyn ScopedTask<'env, R> + 'env>>),
    pub(crate)query_state: &'poll mut dyn FnMut(TaskId) -> TaskState,
    pub(crate)subscribe_io_readable: &'poll mut dyn FnMut(TaskId, usize),
    pub(crate)subscribe_sleep_until: &'poll mut dyn FnMut(TaskId, Instant),

    // Cancellation API: single canonical hook.
    pub(crate)request_cancel_escalate_reason: &'poll mut dyn FnMut(TaskId, CancelEscalation, CancelReason),
}

impl<'poll, 'env, R> Cx<'poll, 'env, R> {
    #[inline]
    pub fn id(&self) -> TaskId {
        self.id
    }

    #[inline]
    pub fn wake(&self, id: TaskId) {
        self.wake_tx.send(WakeMsg { from: self.id, to: id });
    }

    #[inline]
    pub fn wake_self(&self) {
        self.wake(self.id);
    }

    /// Yield to the scheduler and reschedule yourself.
    #[inline]
    pub fn yield_now(&self) -> TaskPoll {
        self.wake_self();
        TaskPoll::Pending
    }

    #[inline]
    pub fn wake_sender(&self) -> WakeSender {
        WakeSender { from: self.id, tx: self.wake_tx.clone() }
    }

    #[inline]
    pub fn subscribe_to_finish(&mut self, target_task: TaskId) {
        (self.subscribe_finish)(self.id, target_task);
    }

    #[inline]
    pub fn spawn_later_named<T>(&mut self, name: impl Into<String>, task: T)
    where
        T: ScopedTask<'env, R> + 'env,
    {
        let task: Pin<Box<dyn ScopedTask<'env, R> + 'env>> = Box::pin(task);
        (self.defer_spawn)(self.id, name.into(), task);
    }

    #[inline]
    pub fn task_state(&mut self, id: TaskId) -> TaskState {
        (self.query_state)(id)
    }

    #[inline]
    pub fn wait_readable(&mut self, token: usize) {
        (self.subscribe_io_readable)(self.id, token);
    }

    #[inline]
    pub fn sleep_until(&mut self, deadline: Instant) {
        (self.subscribe_sleep_until)(self.id, deadline);
    }

    #[inline]
    pub fn sleep_for(&mut self, dur: Duration) {
        self.sleep_until(Instant::now() + dur);
    }

    // ---------------- cancellation ----------------

    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.cancel_flag.load(Ordering::Relaxed)
    }

    /// Best-effort cancel (no additional escalation beyond whatever the scope defaults are).
    #[inline]
    pub fn cancel(&mut self, task: TaskId) {
        // “soft” cancel: use DEFAULT but let Scope resolve defaults.
        (self.request_cancel_escalate_reason)(task, CancelEscalation::DEFAULT, CancelReason::UserRequest);
    }

    #[inline]
    pub fn cancel_with_reason(&mut self, task: TaskId, reason: CancelReason) {
        (self.request_cancel_escalate_reason)(task, CancelEscalation::DEFAULT, reason);
    }

    #[inline]
    pub fn cancel_escalate_with(&mut self, task: TaskId, esc: CancelEscalation) {
        (self.request_cancel_escalate_reason)(task, esc, CancelReason::UserRequest);
    }

    #[inline]
    pub fn cancel_escalate_with_reason(&mut self, task: TaskId, esc: CancelEscalation, reason: CancelReason) {
        (self.request_cancel_escalate_reason)(task, esc, reason);
    }
}

/// Yields once, then completes.
pub struct YieldNowTask {
    yielded: bool,
}
impl YieldNowTask {
    pub fn new() -> Self {
        Self { yielded: false }
    }
}
impl<'env, R> ScopedTask<'env, R> for YieldNowTask {
    fn poll(mut self: Pin<&mut Self>, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        if !self.yielded {
            self.yielded = true;
            return cx.yield_now();
        }
        TaskPoll::Ready
    }
}

/// Convenience adapter: poll a closure.
pub struct FnTask<F>(pub F);
impl<F> Unpin for FnTask<F> {}
impl<'env, R, F> ScopedTask<'env, R> for FnTask<F>
where
    F: for<'p> FnMut(&mut Cx<'p, 'env, R>) -> TaskPoll + 'env,
{
    fn poll(mut self: Pin<&mut Self>, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        (self.0)(cx)
    }
}
