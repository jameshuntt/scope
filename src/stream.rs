use crate::{Cx, ScopedTask, TaskPoll};
use std::{pin::Pin, task::Poll};

pub trait ScopedStream<'env, R>: 'env {
    type Item;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Cx<'_, 'env, R>) -> Poll<Option<Self::Item>>;
}

/// Consume a stream, calling `f(item, cx)` once per yielded item.
/// Fairness: at most 1 item per poll.
pub struct StreamForEachTask<S, F> {
    stream: S,
    f: F,
}

impl<S, F> StreamForEachTask<S, F> {
    pub fn new(stream: S, f: F) -> Self {
        Self { stream, f }
    }
}

impl<'env, R, S, F> ScopedTask<'env, R> for StreamForEachTask<S, F>
where
    S: ScopedStream<'env, R> + 'env,
    F: for<'p> FnMut(S::Item, &mut Cx<'p, 'env, R>) + 'env,
{
    fn poll(mut self: Pin<&mut Self>, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        // SAFETY: we never move `stream`/`f` out
        let this = unsafe { self.as_mut().get_unchecked_mut() };

        match unsafe { Pin::new_unchecked(&mut this.stream) }.poll_next(cx) {
            Poll::Pending => TaskPoll::Pending,
            Poll::Ready(None) => TaskPoll::Ready,
            Poll::Ready(Some(item)) => {
                (this.f)(item, cx);
                TaskPoll::Pending
            }
        }
    }
}
