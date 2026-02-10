// src/select.rs (NEW)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Either<A, B> {
    Left(A),
    Right(B),
}

// src/scope.rs
use crate::{
    JoinHandle, task::{
        Cx, JoinError, JoinPoll, ScopedTask, TaskId, TaskPoll,
    }
};
use std::{
    pin::Pin,
    sync::mpsc,
};

// ---------------- join combinators ----------------

pub(crate)struct Select2Task<A, B> {
    pub(super)prod_a: TaskId,
    pub(super)prod_b: TaskId,
    pub(super)a: Option<JoinHandle<A>>,
    pub(super)b: Option<JoinHandle<B>>,
    pub(super)sub_a: bool,
    pub(super)sub_b: bool,
    pub(super)flip: bool,
    pub(super)tx: Option<mpsc::Sender<Result<Either<Result<A, JoinError>, Result<B, JoinError>>, JoinError>>>,
}

impl<'env, R, A, B> ScopedTask<'env, R> for Select2Task<A, B>
where
    A: Send + 'env,
    B: Send + 'env,
{
    fn poll(mut self: Pin<&mut Self>, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        let this = self.as_mut().get_mut();

        for _ in 0..2 {
            if !this.flip {
                if let Some(a) = this.a.as_mut() {
                    match a.poll() {
                        JoinPoll::Pending => {
                            if !this.sub_a {
                                this.sub_a = true;
                                cx.subscribe_to_finish(this.prod_a);
                            }
                        }
                        JoinPoll::Ready(res) => {
                            if let Some(tx) = this.tx.take() {
                                let _ = tx.send(Ok(Either::Left(res)));
                            }
                            this.a = None;
                            this.b = None;
                            return TaskPoll::Ready;
                        }
                    }
                }
            } else {
                if let Some(b) = this.b.as_mut() {
                    match b.poll() {
                        JoinPoll::Pending => {
                            if !this.sub_b {
                                this.sub_b = true;
                                cx.subscribe_to_finish(this.prod_b);
                            }
                        }
                        JoinPoll::Ready(res) => {
                            if let Some(tx) = this.tx.take() {
                                let _ = tx.send(Ok(Either::Right(res)));
                            }
                            this.a = None;
                            this.b = None;
                            return TaskPoll::Ready;
                        }
                    }
                }
            }
            this.flip = !this.flip;
        }

        TaskPoll::Pending
    }
}

pub(crate)struct SelectNTask<T> {
    pub(super)producers: Vec<TaskId>,
    pub(super)joins: Vec<JoinHandle<T>>,
    pub(super)subscribed: Vec<bool>,
    pub(super)rr: usize,
    pub(super)tx: Option<mpsc::Sender<Result<(usize, Result<T, JoinError>), JoinError>>>,
}

impl<'env, R, T> ScopedTask<'env, R> for SelectNTask<T>
where
    T: Send + 'env,
{
    fn poll(mut self: Pin<&mut Self>, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        let this = self.as_mut().get_mut();
        let n = this.joins.len();
        if n == 0 {
            if let Some(tx) = this.tx.take() {
                let _ = tx.send(Err(JoinError::Panicked));
            }
            return TaskPoll::Ready;
        }
//         if n == 0 {
//             if let Some(tx) = this.tx.take() {
//                 drop(tx);
//             }
//             return TaskPoll::Ready;
//         }

        for k in 0..n {
            let i = (this.rr + k) % n;
            match this.joins[i].poll() {
                JoinPoll::Pending => {
                    if !this.subscribed[i] {
                        this.subscribed[i] = true;
                        cx.subscribe_to_finish(this.producers[i]);
                    }
                }
                JoinPoll::Ready(res) => {
                    if let Some(tx) = this.tx.take() {
                        let _ = tx.send(Ok((i, res)));
                    }
                    return TaskPoll::Ready;
                }
            }
        }

        this.rr = (this.rr + 1) % n;
        TaskPoll::Pending
    }
}
