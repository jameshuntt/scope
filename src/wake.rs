//! The wake substrate: how anything, inside or outside the scope, wakes a task.
//!
//! Two things live here:
//!
//! 1. [`WakeBus`] — a tiny std-only `Mutex<VecDeque> + Condvar` channel carrying
//!    reason-tagged [`SchedulerMsg`]s. Being a plain `Mutex`/`Condvar` it is
//!    also what `--cfg loom` swaps out to model every wake interleaving. The
//!    scheduler takes everything queued in one lock (it trades its empty buffer
//!    for the queue), and a send only signals the condvar when a receiver is
//!    blocked on it.
//!
//! 2. [`scheduler_waker`] — a **real `std::task::Waker`** minted from an
//!    `Arc<WakeSignal>` through `waker_tables`, which builds the waker's vtable
//!    from `WakeSignal` itself, so it cannot run another type's functions. Hand it to an
//!    IO reactor, a timer thread, or any `Future`-shaped code and, when fired,
//!    it enqueues a `Wake { id, reason }` for the scheduler.
//!
//! This module is the *only* place `unsafe` is allowed in the crate, and all of
//! it is the audited `waker_tables` refcount plumbing.
#![allow(unsafe_code)]

use crate::ids::TaskId;
use crate::sync::{lock, Arc, Condvar, Mutex};
use std::collections::VecDeque;
#[cfg(not(loom))]
use std::task::Waker;
#[cfg(any(test, not(loom)))]
use {crate::sync::MutexGuard, std::time::Duration};

// The real-Waker machinery is a std concept and is not what loom model-checks;
// under `--cfg loom` only WakeBus + the message types compile.
#[cfg(not(loom))]
use waker_tables::{derive_arc_unsafe_wakeable, waker_from_arc};

/// Why a task is being woken. Carried on every wake for tracing and for
/// reason-aware scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeReason {
    /// First poll after spawning.
    Spawn,
    /// `Cx::wake`, `wake_self` or `yield_now`.
    Manual,
    /// A sleep deadline passed.
    Timer,
    /// A cancellation was requested, so the task can observe it.
    Cancel,
    /// A task this one subscribed to finished.
    Join,
    /// An external `Waker` fired.
    Io,
    /// A message arrived in the task mailbox.
    Inbox,
    /// The scope is shutting down.
    Shutdown,
}

/// A control message on the wake bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerMsg {
    /// Wake `id` for `reason`.
    Wake {
        /// The task to wake.
        id: TaskId,
        /// Why it is being woken.
        reason: WakeReason,
    },
}

/// What the bus lock guards.
struct Queue {
    msgs: VecDeque<SchedulerMsg>,
    /// Receivers blocked in `cv.wait_timeout` right now. It changes only under
    /// the lock, and a receiver counts itself before its wait releases the lock,
    /// so a sender that reads 0 knows no receiver can be between its empty
    /// check and its wait: there is nobody to notify.
    waiting: usize,
}

struct Inner {
    queue: Mutex<Queue>,
    cv: Condvar,
}

/// Cloneable, `Send + Sync` handle to the scheduler's wake queue.
#[derive(Clone)]
pub struct WakeBus {
    inner: Arc<Inner>,
}

/// The result of a bounded wait on the bus.
#[derive(Debug)]
pub enum WakeRecv {
    /// A message arrived.
    Msg(SchedulerMsg),
    /// The wait expired with nothing queued.
    Timeout,
}

impl WakeBus {
    /// An empty bus.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                queue: Mutex::new(Queue { msgs: VecDeque::new(), waiting: 0 }),
                cv: Condvar::new(),
            }),
        }
    }

    #[inline]
    /// Enqueue a message and notify a blocked receiver.
    pub fn send(&self, msg: SchedulerMsg) {
        let mut q = lock(&self.inner.queue);
        q.msgs.push_back(msg);
        if q.waiting > 0 {
            self.inner.cv.notify_one();
        }
    }

    #[inline]
    /// Enqueue `Wake { id, reason }`.
    pub fn wake(&self, id: TaskId, reason: WakeReason) {
        self.send(SchedulerMsg::Wake { id, reason });
    }

    #[inline]
    /// Dequeue the oldest message without blocking.
    pub fn try_recv(&self) -> Option<SchedulerMsg> {
        lock(&self.inner.queue).msgs.pop_front()
    }

    #[inline]
    /// Whether nothing is queued.
    pub fn is_empty(&self) -> bool {
        lock(&self.inner.queue).msgs.is_empty()
    }

    /// Move every queued message onto the back of `into`, oldest first, under
    /// one lock. When `into` is empty (the scheduler's case) the two buffers
    /// just trade places, so the lock is held for a swap and, once both have
    /// grown, neither allocates again.
    #[inline]
    pub(crate) fn take_all(&self, into: &mut VecDeque<SchedulerMsg>) {
        let mut q = lock(&self.inner.queue);
        if into.is_empty() {
            std::mem::swap(&mut q.msgs, into);
        } else {
            into.append(&mut q.msgs);
        }
    }

    /// Lock the queue and, if it is empty, wait once on the condvar until a
    /// send notifies or `timeout` passes (a spurious wake also ends the wait).
    /// The queue may still be empty in the returned guard. Under loom the wait
    /// never times out, so a missed notify shows up as a deadlock.
    #[cfg(any(test, not(loom)))]
    fn wait_nonempty(&self, timeout: Duration) -> MutexGuard<'_, Queue> {
        let mut q = lock(&self.inner.queue);
        if q.msgs.is_empty() {
            q.waiting += 1;
            q = match self.inner.cv.wait_timeout(q, timeout) {
                Ok((q, _)) => q,
                Err(poisoned) => poisoned.into_inner().0,
            };
            q.waiting -= 1;
        }
        q
    }

    /// Block until something is queued or `timeout` passes, and leave it on
    /// the bus for the next drain. Returns at once if something is queued.
    #[cfg(not(loom))]
    pub(crate) fn wait_timeout(&self, timeout: Duration) {
        drop(self.wait_nonempty(timeout));
    }

    /// Blocking wait bounded by `timeout`. Not compiled under loom (which has
    /// no wall clock); the loom tests below drive the same wait through
    /// `wait_nonempty`.
    #[cfg(not(loom))]
    pub fn recv_timeout(&self, timeout: Duration) -> WakeRecv {
        match self.wait_nonempty(timeout).msgs.pop_front() {
            Some(m) => WakeRecv::Msg(m),
            None => WakeRecv::Timeout,
        }
    }
}

impl Default for WakeBus {
    fn default() -> Self {
        Self::new()
    }
}

/// The payload behind a real `Waker`: which task to wake, why, and where to
/// route it. `Send + Sync + 'static` (required by `waker_tables`) because
/// `WakeBus` is `Arc`-backed and `TaskId`/`WakeReason` are `Copy`.
#[cfg(not(loom))]
pub struct WakeSignal {
    id: TaskId,
    reason: WakeReason,
    bus: WakeBus,
}

#[cfg(not(loom))]
impl WakeSignal {
    fn emit(self: std::sync::Arc<Self>) {
        self.bus.wake(self.id, self.reason);
    }
}

#[cfg(not(loom))]
derive_arc_unsafe_wakeable!(WakeSignal, emit);

/// Mint a real `std::task::Waker` that, when woken, enqueues
/// `Wake { id, reason }` on `bus`. This is the integration point for anything
/// outside the scope.
#[cfg(not(loom))]
pub fn scheduler_waker(id: TaskId, reason: WakeReason, bus: WakeBus) -> Waker {
    waker_from_arc(std::sync::Arc::new(WakeSignal { id, reason, bus }))
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    #[test]
    fn bus_is_fifo() {
        let bus = WakeBus::new();
        let a = TaskId::new(0, 0);
        let b = TaskId::new(1, 0);
        bus.wake(a, WakeReason::Spawn);
        bus.wake(b, WakeReason::Manual);
        assert_eq!(bus.try_recv(), Some(SchedulerMsg::Wake { id: a, reason: WakeReason::Spawn }));
        assert_eq!(bus.try_recv(), Some(SchedulerMsg::Wake { id: b, reason: WakeReason::Manual }));
        assert_eq!(bus.try_recv(), None);
    }

    #[test]
    fn real_waker_routes_back_to_bus() {
        let bus = WakeBus::new();
        let id = TaskId::new(7, 3);
        let waker = scheduler_waker(id, WakeReason::Io, bus.clone());
        // A clone must wake the same task (exercises clone_ptr/drop_ptr).
        let w2 = waker.clone();
        w2.wake();
        waker.wake_by_ref();
        assert_eq!(bus.try_recv(), Some(SchedulerMsg::Wake { id, reason: WakeReason::Io }));
        assert_eq!(bus.try_recv(), Some(SchedulerMsg::Wake { id, reason: WakeReason::Io }));
        assert_eq!(bus.try_recv(), None);
    }

    #[test]
    fn take_all_keeps_fifo_order_behind_leftovers() {
        let bus = WakeBus::new();
        let w = |i| SchedulerMsg::Wake { id: TaskId::new(i, 0), reason: WakeReason::Manual };
        let mut buf = VecDeque::new();

        bus.send(w(0));
        bus.send(w(1));
        bus.take_all(&mut buf);
        assert_eq!(buf, [w(0), w(1)]);
        assert!(bus.is_empty());

        // A batch not fully applied stays in front of anything sent since.
        assert_eq!(buf.pop_front(), Some(w(0)));
        bus.send(w(2));
        bus.take_all(&mut buf);
        assert_eq!(buf, [w(1), w(2)]);

        // The empty buffer the bus got back still takes sends.
        buf.clear();
        bus.send(w(3));
        assert_eq!(bus.try_recv(), Some(w(3)));
        bus.take_all(&mut buf);
        assert!(buf.is_empty());
    }

    #[test]
    fn a_blocked_receiver_is_woken_by_a_send_from_another_thread() {
        let bus = WakeBus::new();
        let tx = bus.clone();
        let id = TaskId::new(2, 0);
        let sender = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            tx.wake(id, WakeReason::Io);
        });
        let start = std::time::Instant::now();
        let mut got = None;
        while got.is_none() && start.elapsed() < Duration::from_secs(30) {
            if let WakeRecv::Msg(m) = bus.recv_timeout(Duration::from_secs(30)) {
                got = Some(m);
            }
        }
        sender.join().unwrap();
        assert_eq!(got, Some(SchedulerMsg::Wake { id, reason: WakeReason::Io }));
        assert!(start.elapsed() < Duration::from_secs(10), "the send did not wake the receiver");
    }
}

/// Loom model of the blocked-receiver handoff: `send` skips `notify_one` when it
/// sees no receiver counted as waiting, and these prove no interleaving loses
/// the wake. Loom's `wait_timeout` never times out, so a lost notify leaves the
/// receiver blocked forever and loom fails the model as a deadlock.
///
/// Run:  RUSTFLAGS="--cfg loom" cargo test --lib --release
#[cfg(all(test, loom))]
mod loom_tests {
    use super::*;

    fn recv_blocking(bus: &WakeBus) -> SchedulerMsg {
        loop {
            if let Some(m) = bus.wait_nonempty(Duration::MAX).msgs.pop_front() {
                return m;
            }
        }
    }

    #[test]
    fn a_receiver_blocked_on_an_empty_bus_always_gets_the_send() {
        loom::model(|| {
            let bus = WakeBus::new();
            let tx = bus.clone();
            let id = TaskId::new(5, 0);
            let producer = loom::thread::spawn(move || tx.wake(id, WakeReason::Io));
            assert_eq!(recv_blocking(&bus), SchedulerMsg::Wake { id, reason: WakeReason::Io });
            producer.join().unwrap();
        });
    }

    #[test]
    fn two_senders_two_blocking_receives_lose_nothing() {
        loom::model(|| {
            let bus = WakeBus::new();
            let (a, b) = (TaskId::new(0, 0), TaskId::new(1, 0));
            let tx_a = bus.clone();
            let tx_b = bus.clone();
            let pa = loom::thread::spawn(move || tx_a.wake(a, WakeReason::Timer));
            let pb = loom::thread::spawn(move || tx_b.wake(b, WakeReason::Inbox));
            let mut got = [recv_blocking(&bus), recv_blocking(&bus)];
            got.sort_by_key(|SchedulerMsg::Wake { id, .. }| *id);
            assert_eq!(
                got,
                [
                    SchedulerMsg::Wake { id: a, reason: WakeReason::Timer },
                    SchedulerMsg::Wake { id: b, reason: WakeReason::Inbox },
                ]
            );
            pa.join().unwrap();
            pb.join().unwrap();
            assert!(bus.is_empty());
        });
    }
}
