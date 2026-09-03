//! Optional scheduler tracing. A [`Tracer`] is a set of no-op-by-default hooks;
//! the scope calls them at interesting moments. Handy for tests
//! ([`BufferTracer`]) and for determinism checks: two runs of the same program
//! must emit identical traces.

use crate::cancel::CancelReason;
use crate::ids::TaskId;
use crate::task::TaskState;
use crate::wake::WakeReason;
use std::time::Duration;

/// Hooks the scope calls at scheduling events. Every method is a no-op by default;
/// implement the ones you want.
pub trait Tracer {
    /// A scheduler round is starting.
    fn on_round_begin(&mut self, _round: u64) {}
    /// A task was spawned.
    fn on_spawn(&mut self, _id: TaskId, _name: &str) {}
    /// A wake was dequeued for a task.
    fn on_wake(&mut self, _id: TaskId, _reason: WakeReason) {}
    /// A task is about to be polled; `seq` counts polls across the run.
    fn on_poll_begin(&mut self, _seq: u64, _id: TaskId, _name: &str) {}
    /// The poll returned.
    fn on_poll_end(&mut self, _id: TaskId, _name: &str) {}
    /// A task left the scope in `state`.
    fn on_finish(&mut self, _id: TaskId, _name: &str, _state: TaskState) {}
    /// `waiter` asked to be woken when `target` finishes.
    fn on_subscribe_finish(&mut self, _waiter: TaskId, _target: TaskId) {}
    /// A task spawned a child during its poll.
    fn on_defer_spawn(&mut self, _parent: TaskId, _child_name: &str) {}
    /// A task arranged a timer wake this many microseconds from now.
    fn on_sleep_until(&mut self, _id: TaskId, _micros_from_now: u128) {}
    /// A soft cancel was requested with these budgets.
    fn on_cancel_request(
        &mut self,
        _id: TaskId,
        _reason: CancelReason,
        _tick_budget: Option<u64>,
        _time_budget: Option<Duration>,
    ) {
    }
    /// A task was hard-stopped.
    fn on_cancel_hard_stop(&mut self, _id: TaskId, _reason: CancelReason) {}
}

/// Prints to stderr.
pub struct LogTracer;

impl Tracer for LogTracer {
    fn on_spawn(&mut self, id: TaskId, name: &str) {
        eprintln!("[spawn] {id} {name}");
    }
    fn on_wake(&mut self, id: TaskId, reason: WakeReason) {
        eprintln!("[wake] {id} {reason:?}");
    }
    fn on_poll_begin(&mut self, seq: u64, id: TaskId, name: &str) {
        eprintln!("[poll#{seq}] {id} {name}");
    }
    fn on_finish(&mut self, id: TaskId, name: &str, state: TaskState) {
        eprintln!("[done] {id} {name} => {state:?}");
    }
    fn on_cancel_hard_stop(&mut self, id: TaskId, reason: CancelReason) {
        eprintln!("[cancel_now] {id} {reason:?}");
    }
}

/// Buffers trace lines in memory. Determinism check: same workload + same
/// policy ⇒ identical `lines()`.
#[derive(Default)]
pub struct BufferTracer {
    lines: Vec<String>,
}

impl BufferTracer {
    /// An empty buffer.
    pub fn new() -> Self {
        Self { lines: Vec::new() }
    }
    /// The lines recorded so far.
    pub fn lines(&self) -> &[String] {
        &self.lines
    }
    /// The recorded lines, consuming the tracer.
    pub fn into_lines(self) -> Vec<String> {
        self.lines
    }
}

impl Tracer for BufferTracer {
    fn on_spawn(&mut self, id: TaskId, name: &str) {
        self.lines.push(format!("spawn {id} {name}"));
    }
    fn on_wake(&mut self, id: TaskId, reason: WakeReason) {
        self.lines.push(format!("wake {id} {reason:?}"));
    }
    fn on_poll_begin(&mut self, seq: u64, id: TaskId, name: &str) {
        self.lines.push(format!("poll {seq} {id} {name}"));
    }
    fn on_finish(&mut self, id: TaskId, name: &str, state: TaskState) {
        self.lines.push(format!("done {id} {name} {state:?}"));
    }
    fn on_cancel_hard_stop(&mut self, id: TaskId, reason: CancelReason) {
        self.lines.push(format!("cancel {id} {reason:?}"));
    }
}

/// Counts polls per task id — the raw material for fairness scoring.
#[derive(Default)]
pub struct CountingTracer {
    /// Polls per task.
    pub polls: std::collections::BTreeMap<TaskId, u64>,
    /// Polls across all tasks.
    pub total_polls: u64,
    /// Tasks that finished.
    pub finishes: u64,
}

impl CountingTracer {
    /// Zeroed counters.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Tracer for CountingTracer {
    fn on_poll_begin(&mut self, _seq: u64, id: TaskId, _name: &str) {
        *self.polls.entry(id).or_insert(0) += 1;
        self.total_polls += 1;
    }
    fn on_finish(&mut self, _id: TaskId, _name: &str, _state: TaskState) {
        self.finishes += 1;
    }
}
