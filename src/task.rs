//! The scoped task model.
//!
//! A [`ScopedTask`] is polled by `&mut self` (not `Pin`: nothing here is
//! self-referential, so dropping `Pin` costs nothing and lets the whole crate
//! forbid `unsafe`). Every poll receives a [`Cx`] holding `&'poll mut R` — a
//! live, exclusive borrow of the scope's environment. That borrow is what real
//! async Rust cannot safely hand a spawned task; scope can, because polling is
//! cooperative and single-threaded, so only one task touches `R` at a time.

use crate::cancel::{CancelCell, CancelEscalation, CancelReason};
use crate::ids::TaskId;
use crate::policy::QoSClass;
use crate::sync::Arc;
use crate::wake::{WakeBus, WakeReason};
#[cfg(not(loom))]
use crate::wake::scheduler_waker;
#[cfg(not(loom))]
use std::task::Waker;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// What a poll returned.
pub enum TaskPoll {
    /// Not finished; the task arranged a wake, or yielded, and will be polled again.
    Pending,
    /// Finished. The task leaves the scope.
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Where a task is in its life.
pub enum TaskState {
    /// Spawned and not yet finished.
    Alive,
    /// Returned `Ready`.
    Completed,
    /// Hard-stopped by cancellation.
    Cancelled,
    /// Its poll panicked; the panic was caught and isolated.
    Panicked,
}

/// Why a `JoinHandle` produced no value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinError {
    /// The task panicked.
    Panicked,
    /// The task was cancelled, with the reason that won.
    Cancelled(CancelReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// One non-consuming look at a [`crate::JoinHandle`].
pub enum JoinPoll<T> {
    /// The task has not finished yet.
    Pending,
    /// The task finished; the value is taken out of the handle.
    Ready(Result<T, JoinError>),
}

/// A unit of cooperative work living inside a [`crate::Scope`]. It may borrow
/// data of the scope lifetime `'env`, including through the `&mut R` handed to
/// each poll.
pub trait ScopedTask<'env, R>: 'env {
    /// Make progress. Return [`TaskPoll::Pending`] after arranging a wake (or after
    ///     /// [`Cx::yield_now`]), [`TaskPoll::Ready`] when done.
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll;
}

/// Per-poll context. Carries the live environment borrow plus typed hooks back
/// into the scheduler (subscribe-to-finish, deferred spawn, sleep, cancel).
pub struct Cx<'poll, 'env, R> {
    /// The scope environment, borrowed exclusively for this poll.
    pub resources: &'poll mut R,

    pub(crate) id: TaskId,
    pub(crate) bus: WakeBus,
    pub(crate) cancel: Arc<CancelCell>,

    pub(crate) subscribe_finish: &'poll mut dyn FnMut(TaskId, TaskId),
    pub(crate) defer_spawn:
        &'poll mut dyn FnMut(TaskId, String, QoSClass, Box<dyn ScopedTask<'env, R> + 'env>),
    pub(crate) query_state: &'poll mut dyn FnMut(TaskId) -> TaskState,
    pub(crate) subscribe_sleep_until: &'poll mut dyn FnMut(TaskId, Instant),
    pub(crate) request_cancel: &'poll mut dyn FnMut(TaskId, CancelEscalation, CancelReason),
}

impl<'poll, 'env, R> Cx<'poll, 'env, R> {
    #[inline]
    /// This task's id.
    pub fn id(&self) -> TaskId {
        self.id
    }

    /// Enqueue a wake for `id` on the shared bus (reason: `Manual`).
    #[inline]
    pub fn wake(&self, id: TaskId) {
        self.bus.wake(id, WakeReason::Manual);
    }

    #[inline]
    /// Enqueue a wake for `id` with an explicit reason.
    pub fn wake_with(&self, id: TaskId, reason: WakeReason) {
        self.bus.wake(id, reason);
    }

    #[inline]
    /// Enqueue a wake for this task (reason `Manual`).
    pub fn wake_self(&self) {
        self.bus.wake(self.id, WakeReason::Manual);
    }

    /// Reschedule this task on the next round.
    #[inline]
    pub fn yield_now(&self) -> TaskPoll {
        self.wake_self();
        TaskPoll::Pending
    }

    /// A **real `std::task::Waker`** for this task. Hand it to an IO reactor,
    /// timer, or `Future`-shaped code; firing it wakes this task through the
    /// same bus.
    #[cfg(not(loom))]
    #[inline]
    pub fn waker(&self) -> Waker {
        scheduler_waker(self.id, WakeReason::Io, self.bus.clone())
    }

    #[cfg(not(loom))]
    #[inline]
    /// A real `std::task::Waker` that wakes `id` with `reason`.
    pub fn waker_for(&self, id: TaskId, reason: WakeReason) -> Waker {
        scheduler_waker(id, reason, self.bus.clone())
    }

    /// Wake this task when `target` finishes.
    #[inline]
    pub fn subscribe_to_finish(&mut self, target: TaskId) {
        (self.subscribe_finish)(self.id, target);
    }

    /// Spawn a child into the same scope (runs on a later round).
    #[inline]
    pub fn spawn_later_named<T>(&mut self, name: impl Into<String>, task: T)
    where
        T: ScopedTask<'env, R> + 'env,
    {
        self.spawn_later_qos(name, QoSClass::Standard, task);
    }

    #[inline]
    /// Spawn a child into the same scope with an explicit QoS class; it runs on a later round.
    pub fn spawn_later_qos<T>(&mut self, name: impl Into<String>, qos: QoSClass, task: T)
    where
        T: ScopedTask<'env, R> + 'env,
    {
        (self.defer_spawn)(self.id, name.into(), qos, Box::new(task));
    }

    #[inline]
    /// The current state of `id`.
    pub fn task_state(&mut self, id: TaskId) -> TaskState {
        (self.query_state)(id)
    }

    #[inline]
    /// Arrange a `Timer` wake for this task at `deadline`; return `Pending` afterwards.
    pub fn sleep_until(&mut self, deadline: Instant) {
        (self.subscribe_sleep_until)(self.id, deadline);
    }

    #[inline]
    /// [`Cx::sleep_until`] relative to now.
    pub fn sleep_for(&mut self, dur: Duration) {
        self.sleep_until(Instant::now() + dur);
    }

    // ---- cancellation ----

    #[inline]
    /// Whether this task has been asked to stop. A cooperative task checks this and returns `Ready`.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    #[inline]
    /// Why this task was asked to stop.
    pub fn cancel_reason(&self) -> CancelReason {
        self.cancel.reason()
    }

    #[inline]
    /// Request cancellation of `task` with the scope default escalation and reason `UserRequest`.
    pub fn cancel(&mut self, task: TaskId) {
        (self.request_cancel)(task, CancelEscalation::DEFAULT, CancelReason::UserRequest);
    }

    #[inline]
    /// Request cancellation of `task` with the default escalation and an explicit reason.
    pub fn cancel_with_reason(&mut self, task: TaskId, reason: CancelReason) {
        (self.request_cancel)(task, CancelEscalation::DEFAULT, reason);
    }

    #[inline]
    /// Request cancellation of `task` with an explicit escalation and reason.
    pub fn cancel_escalate_with_reason(
        &mut self,
        task: TaskId,
        esc: CancelEscalation,
        reason: CancelReason,
    ) {
        (self.request_cancel)(task, esc, reason);
    }
}

/// Yields once, then completes. Handy for fairness tests.
pub struct YieldNowTask {
    yielded: bool,
}

impl YieldNowTask {
    /// A task that will yield once.
    pub fn new() -> Self {
        Self { yielded: false }
    }
}

impl Default for YieldNowTask {
    fn default() -> Self {
        Self::new()
    }
}

impl<'env, R> ScopedTask<'env, R> for YieldNowTask {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        if !self.yielded {
            self.yielded = true;
            return cx.yield_now();
        }
        TaskPoll::Ready
    }
}

/// Adapter: drive a closure as a task.
pub struct FnTask<F>(pub F);

impl<'env, R, F> ScopedTask<'env, R> for FnTask<F>
where
    F: for<'p> FnMut(&mut Cx<'p, 'env, R>) -> TaskPoll + 'env,
{
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        (self.0)(cx)
    }
}
