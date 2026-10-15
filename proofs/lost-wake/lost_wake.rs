//! A wake sent from another thread while `run()` or `run_until()` waits idle has to reach its task.
//! The first test is `a_wake_from_another_thread_during_runs_idle_wait_is_not_lost` from
//! `tests/kernel.rs` at da655cf, unchanged. The second does the same through `run_until()`.
#![cfg(test)]

use scope::{run_scoped, Cx, Scope, ScopedTask, TaskPoll};
use std::time::{Duration, Instant};

// Parks on its first poll and hands its real waker to a thread that fires it
// shortly after. Once woken it hard-stops the long keepalive sleep.
struct WokenFromAThread {
    keepalive: scope::TaskId,
    started: bool,
}
impl<'env, R> ScopedTask<'env, R> for WokenFromAThread {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        if !self.started {
            self.started = true;
            let waker = cx.waker();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(50));
                waker.wake();
            });
            return TaskPoll::Pending;
        }
        cx.cancel_escalate_with_reason(
            self.keepalive,
            scope::CancelEscalation::immediate(),
            scope::CancelReason::UserRequest,
        );
        TaskPoll::Ready
    }
}

#[test]
fn a_wake_from_another_thread_during_runs_idle_wait_is_not_lost() {
    // `run` blocks on the bus until the 10 s sleep is due; the other thread's
    // wake lands during that wait and must still reach the task.
    let start = std::time::Instant::now();
    let mut env = ();
    let report = run_scoped(&mut env, |s| {
        let keepalive = s.sleep("keepalive", Duration::from_secs(10)).task;
        s.spawn_named("woken", WokenFromAThread { keepalive, started: false });
    });
    assert!(report.stalled.is_empty(), "{report:?}");
    assert_eq!((report.completed, report.cancelled), (1, 1), "{report:?}");
    assert!(start.elapsed() < Duration::from_secs(5), "the run waited out the sleep: {report:?}");
}

// Parks on its first poll and hands its real waker to a thread that fires it 50 ms later.
// Finishes on its second poll.
struct ParksOnce {
    started: bool,
}
impl<'env, R> ScopedTask<'env, R> for ParksOnce {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        if self.started {
            return TaskPoll::Ready;
        }
        self.started = true;
        let waker = cx.waker();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            waker.wake();
        });
        TaskPoll::Pending
    }
}

#[test]
fn run_until_keeps_a_wake_that_ends_its_idle_wait() {
    // Nothing else is pending, so `run_until` waits on the bus for up to 3 s.
    // The wake lands at 50 ms; the task should finish right after it.
    let start = Instant::now();
    let mut env = ();
    let mut s = Scope::new(&mut env);
    s.spawn_named("woken", ParksOnce { started: false });
    let finished = s.run_until(Duration::from_secs(3));
    let took = start.elapsed();
    assert!(finished && took < Duration::from_secs(2), "finished={finished} after {took:?}");
}
