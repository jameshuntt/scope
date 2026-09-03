//! A cooperative sleep task backed by the scope timer wheel.

use crate::task::{Cx, ScopedTask, TaskPoll};
use std::time::{Duration, Instant};

/// A task that completes once `dur` has elapsed, parked on the scope timer in between.
pub struct SleepTask {
    dur: Duration,
    deadline: Option<Instant>,
    armed: bool,
}

impl SleepTask {
    /// A sleep of `dur`, armed on its first poll.
    pub fn new(dur: Duration) -> Self {
        Self { dur, deadline: None, armed: false }
    }
}

impl<'env, R> ScopedTask<'env, R> for SleepTask {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        let now = Instant::now();
        let dl = *self.deadline.get_or_insert(now + self.dur);
        if now >= dl {
            return TaskPoll::Ready;
        }
        if !self.armed {
            self.armed = true;
            cx.sleep_until(dl);
        }
        TaskPoll::Pending
    }
}
