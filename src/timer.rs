//! Cooperative sleep tasks: [`SleepTask`] on the wall clock, [`TickSleepTask`] on the logical clock.

use crate::clock::{Tick, TickDelta};
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

/// A task that completes at a tick of the logical clock (see [`crate::clock`]), parked on the
/// scope's tick timers in between. Beside [`SleepTask`], not instead of it: it never reads the
/// wall clock, so the round it completes in depends only on the policy and the order of wakes.
pub struct TickSleepTask {
    wait: TickWait,
    deadline: Option<u64>,
    armed: bool,
}

#[derive(Clone, Copy)]
enum TickWait {
    For(TickDelta),
    Until(Tick),
}

impl TickSleepTask {
    /// Completes `delta` rounds after its first poll; with a `delta` of 0, at its first poll. A
    /// round past `u64::MAX` is `u64::MAX`, where the clock stops ([`crate::clock`]).
    pub fn new(delta: TickDelta) -> Self {
        Self {
            wait: TickWait::For(delta),
            deadline: None,
            armed: false,
        }
    }

    /// Completes in round `tick`, or at its first poll if that round has come.
    pub fn until(tick: Tick) -> Self {
        Self {
            wait: TickWait::Until(tick),
            deadline: None,
            armed: false,
        }
    }
}

impl<'env, R> ScopedTask<'env, R> for TickSleepTask {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        let now = cx.now_tick().0;
        let wait = self.wait;
        let dl = *self.deadline.get_or_insert_with(|| match wait {
            TickWait::For(delta) => now.saturating_add(delta.0),
            TickWait::Until(tick) => tick.0,
        });
        if now >= dl {
            return TaskPoll::Ready;
        }
        if !self.armed {
            self.armed = true;
            cx.sleep_until_tick(Tick(dl));
        }
        TaskPoll::Pending
    }
}
