// src/scope.rs
use crate::{
    JoinHandle, task::{Cx, JoinError, JoinPoll, ScopedTask, TaskId, TaskPoll,}
};
use std::{
    pin::Pin,
    sync::mpsc,
};

// ----------------------------- tick result -----------------------------
pub enum TickResult {
    Progress,
    Idle,
    Done,
}

pub struct MapJoinTask<T, Out, F> {
    pub(crate)producer: TaskId,
    pub(crate)join: Option<JoinHandle<T>>,
    pub(crate)tx: Option<mpsc::Sender<Result<Out, JoinError>>>,
    pub(crate)f: Option<F>,
    pub(crate)subscribed: bool,
}

impl<'env, R, T, Out, F> ScopedTask<'env, R> for MapJoinTask<T, Out, F>
where
    T: Send + 'env,
    Out: Send + 'env,
    F: for<'p> FnOnce(Result<T, JoinError>, &mut Cx<'p, 'env, R>) -> Out + 'env,
{
    fn poll(mut self: Pin<&mut Self>, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        let this = self.as_mut().get_mut();
        let Some(join) = this.join.as_mut() else { return TaskPoll::Ready; };

        match join.poll() {
            JoinPoll::Pending => {
                if !this.subscribed {
                    this.subscribed = true;
                    cx.subscribe_to_finish(this.producer);
                }
                TaskPoll::Pending
            }
            JoinPoll::Ready(res) => {
                let f = this.f.take().expect("map-join f already taken");
                let out = f(res, cx);
                if let Some(tx) = this.tx.take() {
                    let _ = tx.send(Ok(out));
                }
                this.join = None;
                TaskPoll::Ready
            }
        }
    }
}
