// src/timer.rs (NEW)
use crate::{Cx, ScopedTask, TaskPoll};
use std::{pin::Pin, time::{Duration, Instant}};

pub struct SleepTask {
    dur: Duration,
    deadline: Option<Instant>,
    armed: bool,
}

impl SleepTask {
    pub fn new(dur: Duration) -> Self {
        Self { dur, deadline: None, armed: false }
    }
}

impl<'env, R> ScopedTask<'env, R> for SleepTask {
    fn poll(mut self: Pin<&mut Self>, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        let now = Instant::now();
        if self.deadline.is_none() {
            self.deadline = Some(now + self.dur);
        }
        let dl = self.deadline.unwrap();

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