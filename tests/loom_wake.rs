//! Loom model of the cross-thread wake/cancel handoff.
//!
//! Compiled only under `RUSTFLAGS="--cfg loom"`; empty otherwise. Under loom,
//! `scope::sync` resolves to loom's instrumented `Mutex`/`Condvar`/atomics, so
//! the model checker explores every interleaving of a canceller thread and the
//! scheduler thread.
//!
//! Run:  RUSTFLAGS="--cfg loom" cargo test --test loom_wake --release
#![cfg(loom)]

use scope::cancel::{CancelCell, CancelReason};
use scope::ids::TaskId;
use scope::wake::{SchedulerMsg, WakeBus, WakeReason};

/// Invariant the scheduler depends on: if the scheduler observes a `Cancel`
/// wake on the bus, the shared cancel flag is already visible as set. If the
/// flag store used an insufficient ordering, loom would find an interleaving
/// where the wake is delivered but `is_cancelled()` is still false.
#[test]
fn delivered_cancel_wake_implies_flag_visible() {
    loom::model(|| {
        let bus = WakeBus::new();
        let cell = loom::sync::Arc::new(CancelCell::new());
        let id = TaskId::new(0, 0);

        let bus_c = bus.clone();
        let cell_c = cell.clone();
        let canceller = loom::thread::spawn(move || {
            // Order matters: publish the reason/flag, then post the wake.
            cell_c.request(CancelReason::UserRequest);
            bus_c.wake(id, WakeReason::Cancel);
        });

        // Scheduler side: draining a Cancel wake must imply the flag is set.
        if let Some(SchedulerMsg::Wake { reason: WakeReason::Cancel, .. }) = bus.try_recv() {
            assert!(cell.is_cancelled(), "cancel wake delivered but flag not visible");
            assert_eq!(cell.reason(), CancelReason::UserRequest);
        }

        canceller.join().unwrap();
        // After the join, the flag is unconditionally visible.
        assert!(cell.is_cancelled());
    });
}

/// No wake is lost: after both threads run, exactly the posted wake is present
/// (or was already drained). Explores the producer/consumer race on the bus.
#[test]
fn wake_is_never_lost() {
    loom::model(|| {
        let bus = WakeBus::new();
        let id = TaskId::new(3, 1);

        let bus_c = bus.clone();
        let producer = loom::thread::spawn(move || {
            bus_c.wake(id, WakeReason::Timer);
        });

        let first = bus.try_recv();
        producer.join().unwrap();
        let second = bus.try_recv();

        // The wake is delivered exactly once, whichever side of the race it lands.
        let seen = first.is_some() as u32 + second.is_some() as u32;
        assert_eq!(seen, 1, "wake delivered exactly once");
    });
}
