use crate::{
    CancelEscalation, CancelReason, JoinHandle, ops::map::MapJoinTask, task::{Cx, JoinError, JoinPoll, ScopedTask, TaskId, TaskPoll,}
};
use std::{
    pin::Pin,
    sync::mpsc,
};

pub(crate)struct JoinAllTask<T> {
    pub(crate)producers: Vec<TaskId>,
    pub(crate)joins: Vec<JoinHandle<T>>,
    pub(crate)results: Vec<Option<Result<T, JoinError>>>,
    pub(crate)subscribed: Vec<bool>,
    pub(crate)tx: Option<mpsc::Sender<Result<Vec<Result<T, JoinError>>, JoinError>>>,
}

impl<'env, R, T> ScopedTask<'env, R> for JoinAllTask<T>
where
    T: Send + 'env,
{
    fn poll(mut self: Pin<&mut Self>, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        let this = self.as_mut().get_mut();

        for i in 0..this.joins.len() {
            if this.results[i].is_some() {
                continue;
            }
            match this.joins[i].poll() {
                JoinPoll::Pending => {
                    if !this.subscribed[i] {
                        this.subscribed[i] = true;
                        cx.subscribe_to_finish(this.producers[i]);
                    }
                }
                // JoinPoll::Ready(res) => {
                //     this.results[i] = Some(res);
                // }
//                 JoinPoll::Ready(res) => {
//     // ⚡️ THE CRITICAL CHANGE:
//     if let Err(e) = &res {
//         // If we hit an error, we don't wait for anyone else.
//         if let Some(tx) = this.tx.take() {
//             // We report the error immediately.
//             let _ = tx.send(Err(e.clone())); 
//         }
//         // We tell the executor this JoinAll task is finished (failed).
//         return TaskPoll::Ready; 
//     }
//     
//     // If it's an Ok, we store it and keep going.
//     this.results[i] = Some(res);
// }
/* ... inside the loop in JoinAllTask::poll ... */
JoinPoll::Ready(res) => {
    if let Err(e) = &res {
        // 1. Tell the person waiting for JoinAll that we failed
        if let Some(tx) = this.tx.take() {
            let _ = tx.send(Err(e.clone()));
        }

        // 2. 🚨 ACTUALLY ESCALATE
        // This tells the executor to flip the cancel_flag for EVERYTHING in the scope
        (cx.request_cancel_escalate_reason)(
            cx.id, 
            CancelEscalation::DEFAULT, // Use whatever constructor your struct has for Scope-level
            CancelReason::ShortCircuit         // Priority 20: This will override napping tasks
        );

        return TaskPoll::Ready; 
    }
    this.results[i] = Some(res);
}
            }
        }

        if this.results.iter().all(|r| r.is_some()) {
            let out = this
                .results
                .iter_mut()
                .map(|r| r.take().unwrap())
                .collect::<Vec<_>>();
            if let Some(tx) = this.tx.take() {
                let _ = tx.send(Ok(out));
            }
            return TaskPoll::Ready;
        }

        TaskPoll::Pending
    }
}

// JoinAllOk: joins Result<T,E> and stops on first Err(E) or panic.
#[derive(Debug)]
pub enum JoinAllOkError<E> {
    Panicked { index: usize },
    Err { index: usize, err: E },
    JoinAllFailed(JoinError)
}

pub(crate)struct JoinAllOkTask<T, E> {
    pub(crate)producers: Vec<TaskId>,
    pub(crate)joins: Vec<JoinHandle<Result<T, E>>>,
    pub(crate)subscribed: Vec<bool>,
    pub(crate)vals: Vec<Option<T>>,
    pub(crate)tx: Option<mpsc::Sender<Result<Result<Vec<T>, JoinAllOkError<E>>, JoinError>>>,
}

impl<T, Out, F> Unpin for MapJoinTask<T, Out, F> {}

impl<T> Unpin for JoinAllTask<T> {}

impl<T, E> Unpin for JoinAllOkTask<T, E> {}


impl<'env, R, T, E> ScopedTask<'env, R> for JoinAllOkTask<T, E>
where
    T: Send + 'env,
    E: Send + 'env,
{
    fn poll(mut self: Pin<&mut Self>, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        let this: &mut JoinAllOkTask<T, E> = self.as_mut().get_mut();
        // works but unsafe is not my forte
        // let this: &mut JoinAllOkTask<T, E> = unsafe { self.as_mut().get_unchecked_mut() };

        for i in 0..this.joins.len() {
            if this.vals[i].is_some() {
                continue;
            }

            match this.joins[i].poll() {
                JoinPoll::Pending => {
                    if !this.subscribed[i] {
                        this.subscribed[i] = true;
                        cx.subscribe_to_finish(this.producers[i]);
                    }
                }
                JoinPoll::Ready(res) => match res {
                    Err(JoinError::Panicked) => {
                        if let Some(tx) = this.tx.take() {
                            let _ = tx.send(Ok(Err(JoinAllOkError::Panicked { index: i })));
                        }
                        return TaskPoll::Ready;
                    }
                    Err(JoinError::Cancelled(r)) => {
                        if let Some(tx) = this.tx.take() {
                            let _ = tx.send(Err(JoinError::Cancelled(r)));
                        }
                        return TaskPoll::Ready;
                    }
                    Ok(Ok(v)) => this.vals[i] = Some(v),
                    Ok(Err(e)) => {
                        if let Some(tx) = this.tx.take() {
                            let _ = tx.send(Ok(Err(JoinAllOkError::Err { index: i, err: e })));
                        }
                        return TaskPoll::Ready;
                    }
                },
            }
        }

        if this.vals.iter().all(|v| v.is_some()) {
            let out = this.vals.iter_mut().map(|v| v.take().unwrap()).collect::<Vec<_>>();
            if let Some(tx) = this.tx.take() {
                let _ = tx.send(Ok(Ok(out)));
            }
            return TaskPoll::Ready;
        }

        TaskPoll::Pending
    }
}
