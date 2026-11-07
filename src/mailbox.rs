//! Typed mailboxes: deliver a message *into* a running scoped task.
//!
//! A mailbox is an `Arc`-shared typed queue plus a wake through the existing
//! [`WakeBus`]. The scheduler never stores heterogeneous message types: the
//! queue lives beside the task, not in the slab, so this stays fully
//! `#![deny(unsafe_code)]` and works with scoped, non-`'static` messages.
//!
//! ```ignore
//! let (id, tx) = scope.spawn_with_mailbox("sink", |rx| Summer { rx, total: 0 });
//! tx.send(10);            // from anywhere holding the sender (incl. other threads)
//! tx.send(32);
//! scope.run();
//! ```
//!
//! [`Mailbox`] / [`MailboxSender`] are unbounded: `send` always enqueues. A
//! [`BoundedMailbox`] / [`BoundedSender`] pair, made by
//! `Scope::spawn_with_bounded_mailbox`, holds at most the capacity it was made
//! with. Its queue is allocated once, at that capacity. `try_send` enqueues and
//! wakes exactly like `send`, or gives the message back: [`TrySendError::Full`]
//! when the capacity is taken, [`TrySendError::Closed`] once the receiving task
//! is gone (its `BoundedMailbox` dropped, which happens when the task completes,
//! is cancelled or panics) or closed it. A refused message is not enqueued, no
//! wake is sent, nothing is allocated. Receiving frees capacity at once.

use crate::ids::TaskId;
use crate::sync::{lock, Arc, Mutex};
use crate::wake::{WakeBus, WakeReason};
use std::collections::VecDeque;

/// The read end, held *inside* the receiving task. Drain it each poll.
pub struct Mailbox<M> {
    queue: Arc<Mutex<VecDeque<M>>>,
}

impl<M> Mailbox<M> {
    /// Pop the next message, if any.
    pub fn try_recv(&self) -> Option<M> {
        lock(&self.queue).pop_front()
    }

    /// Drain every currently-queued message.
    pub fn drain(&self) -> Vec<M> {
        let mut q = lock(&self.queue);
        q.drain(..).collect()
    }

    /// Whether nothing is queued.
    pub fn is_empty(&self) -> bool {
        lock(&self.queue).is_empty()
    }

    /// Number of queued messages.
    pub fn len(&self) -> usize {
        lock(&self.queue).len()
    }
}

/// The write end. Cloneable (multi-producer) and `Send + Sync` when `M: Send`,
/// so other tasks — or other threads — can post into the task and wake it.
pub struct MailboxSender<M> {
    queue: Arc<Mutex<VecDeque<M>>>,
    bus: WakeBus,
    target: TaskId,
}

impl<M> MailboxSender<M> {
    /// Enqueue `msg` and wake the receiving task (reason [`WakeReason::Inbox`]).
    pub fn send(&self, msg: M) {
        lock(&self.queue).push_back(msg);
        self.bus.wake(self.target, WakeReason::Inbox);
    }

    /// The task this sender delivers to.
    pub fn target(&self) -> TaskId {
        self.target
    }
}

impl<M> Clone for MailboxSender<M> {
    fn clone(&self) -> Self {
        Self { queue: self.queue.clone(), bus: self.bus.clone(), target: self.target }
    }
}

/// Internal constructor used by `Scope::spawn_with_mailbox`. The sender is bound
/// to the bus + task id separately (via [`sender`]) once the task is spawned.
pub(crate) fn channel<M>() -> (Arc<Mutex<VecDeque<M>>>, Mailbox<M>) {
    let queue: Arc<Mutex<VecDeque<M>>> = Arc::new(Mutex::new(VecDeque::new()));
    (queue.clone(), Mailbox { queue })
}

pub(crate) fn sender<M>(
    queue: Arc<Mutex<VecDeque<M>>>,
    bus: WakeBus,
    target: TaskId,
) -> MailboxSender<M> {
    MailboxSender { queue, bus, target }
}

/// Why [`BoundedSender::try_send`] did not enqueue a message. The message comes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrySendError<M> {
    /// The mailbox holds its capacity of messages.
    Full(M),
    /// The receiving task is gone, or closed its mailbox.
    Closed(M),
}

impl<M> TrySendError<M> {
    /// The message that was not sent.
    pub fn into_inner(self) -> M {
        match self {
            TrySendError::Full(m) | TrySendError::Closed(m) => m,
        }
    }
}

/// What a bounded mailbox's two ends share.
pub(crate) struct Bounded<M> {
    msgs: VecDeque<M>,
    capacity: usize,
    closed: bool,
}

/// The read end of a bounded mailbox, held *inside* the receiving task. Dropping it (the task
/// finishing, by any path) closes the mailbox and drops what is still queued.
pub struct BoundedMailbox<M> {
    shared: Arc<Mutex<Bounded<M>>>,
}

impl<M> BoundedMailbox<M> {
    /// Pop the next message, if any. Frees one place.
    pub fn try_recv(&self) -> Option<M> {
        lock(&self.shared).msgs.pop_front()
    }

    /// Drain every currently-queued message. Frees every place.
    pub fn drain(&self) -> Vec<M> {
        lock(&self.shared).msgs.drain(..).collect()
    }

    /// Whether nothing is queued.
    pub fn is_empty(&self) -> bool {
        lock(&self.shared).msgs.is_empty()
    }

    /// Number of queued messages.
    pub fn len(&self) -> usize {
        lock(&self.shared).msgs.len()
    }

    /// The most messages it holds.
    pub fn capacity(&self) -> usize {
        lock(&self.shared).capacity
    }

    /// Refuse every later send with `Closed`. Messages already queued stay readable.
    pub fn close(&self) {
        lock(&self.shared).closed = true;
    }
}

impl<M> Drop for BoundedMailbox<M> {
    fn drop(&mut self) {
        // Take the messages out under the lock and drop them after it.
        let rest = {
            let mut q = lock(&self.shared);
            q.closed = true;
            std::mem::take(&mut q.msgs)
        };
        drop(rest);
    }
}

/// The write end of a bounded mailbox. Cloneable, and `Send + Sync` when `M: Send`, like
/// [`MailboxSender`].
pub struct BoundedSender<M> {
    shared: Arc<Mutex<Bounded<M>>>,
    bus: WakeBus,
    target: TaskId,
}

impl<M> BoundedSender<M> {
    /// Enqueue `msg` and wake the receiving task (reason [`WakeReason::Inbox`]), or give it back:
    /// `Full` when the mailbox holds its capacity, `Closed` when the receiver is gone. A refused
    /// message enqueues nothing, wakes nothing and allocates nothing.
    pub fn try_send(&self, msg: M) -> Result<(), TrySendError<M>> {
        {
            let mut q = lock(&self.shared);
            if q.closed {
                return Err(TrySendError::Closed(msg));
            }
            if q.msgs.len() >= q.capacity {
                return Err(TrySendError::Full(msg));
            }
            q.msgs.push_back(msg);
        }
        self.bus.wake(self.target, WakeReason::Inbox);
        Ok(())
    }

    /// Number of queued messages.
    pub fn len(&self) -> usize {
        lock(&self.shared).msgs.len()
    }

    /// Whether nothing is queued.
    pub fn is_empty(&self) -> bool {
        lock(&self.shared).msgs.is_empty()
    }

    /// The most messages the mailbox holds.
    pub fn capacity(&self) -> usize {
        lock(&self.shared).capacity
    }

    /// Places left: how many `try_send`s would succeed now if nothing else sent or received.
    /// Zero once the mailbox is closed.
    pub fn free(&self) -> usize {
        let q = lock(&self.shared);
        if q.closed {
            0
        } else {
            q.capacity.saturating_sub(q.msgs.len())
        }
    }

    /// Whether the receiver is gone or closed the mailbox.
    pub fn is_closed(&self) -> bool {
        lock(&self.shared).closed
    }

    /// The task this sender delivers to.
    pub fn target(&self) -> TaskId {
        self.target
    }
}

impl<M> Clone for BoundedSender<M> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
            bus: self.bus.clone(),
            target: self.target,
        }
    }
}

/// Internal constructor used by `Scope::spawn_with_bounded_mailbox`: the queue, allocated once
/// for `capacity` messages.
pub(crate) fn bounded_channel<M>(capacity: usize) -> (Arc<Mutex<Bounded<M>>>, BoundedMailbox<M>) {
    let shared = Arc::new(Mutex::new(Bounded {
        msgs: VecDeque::with_capacity(capacity),
        capacity,
        closed: false,
    }));
    (shared.clone(), BoundedMailbox { shared })
}

pub(crate) fn bounded_sender<M>(
    shared: Arc<Mutex<Bounded<M>>>,
    bus: WakeBus,
    target: TaskId,
) -> BoundedSender<M> {
    BoundedSender {
        shared,
        bus,
        target,
    }
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;
    use crate::wake::SchedulerMsg;

    fn pair(capacity: usize) -> (BoundedSender<u32>, BoundedMailbox<u32>, WakeBus) {
        let bus = WakeBus::new();
        let (shared, rx) = bounded_channel(capacity);
        (
            bounded_sender(shared, bus.clone(), TaskId::new(4, 0)),
            rx,
            bus,
        )
    }

    #[test]
    fn a_refused_send_allocates_nothing_and_wakes_nothing() {
        let (tx, rx, bus) = pair(3);
        let allocated = lock(&rx.shared).msgs.capacity();
        assert!(allocated >= 3);
        for round in 0..50u32 {
            for i in 0..3 {
                assert_eq!(tx.try_send(round * 10 + i), Ok(()));
            }
            assert_eq!(tx.try_send(99), Err(TrySendError::Full(99)));
            assert_eq!(tx.free(), 0);
            assert_eq!(
                lock(&rx.shared).msgs.capacity(),
                allocated,
                "the queue never grew"
            );
            assert_eq!(rx.drain(), [round * 10, round * 10 + 1, round * 10 + 2]);
        }
        // One Inbox wake per accepted message, none for a refusal.
        let mut wakes = 0;
        while let Some(SchedulerMsg::Wake { reason, .. }) = bus.try_recv() {
            assert_eq!(reason, WakeReason::Inbox);
            wakes += 1;
        }
        assert_eq!(wakes, 150);
    }

    #[test]
    fn dropping_the_read_end_closes_and_frees_the_queue() {
        let (tx, rx, _bus) = pair(2);
        assert_eq!(tx.try_send(1), Ok(()));
        drop(rx);
        assert!(tx.is_closed());
        assert_eq!(
            tx.len(),
            0,
            "queued messages were dropped with the read end"
        );
        assert_eq!(tx.try_send(2), Err(TrySendError::Closed(2)));
        assert_eq!(tx.free(), 0);
    }
}

/// Loom model of a bounded mailbox shared across threads: the bound holds and nothing is lost or
/// delivered twice in any interleaving, and a delivered `Inbox` wake means the message is there.
///
/// Run:  RUSTFLAGS="--cfg loom" cargo test --lib --release
#[cfg(all(test, loom))]
mod loom_tests {
    use super::*;
    use crate::wake::SchedulerMsg;

    #[test]
    fn a_delivered_inbox_wake_implies_the_message_is_queued() {
        loom::model(|| {
            let bus = WakeBus::new();
            let id = TaskId::new(1, 0);
            let (shared, rx) = bounded_channel::<u32>(1);
            let tx = bounded_sender(shared, bus.clone(), id);
            let producer = loom::thread::spawn(move || tx.try_send(7));
            if let Some(SchedulerMsg::Wake {
                reason: WakeReason::Inbox,
                ..
            }) = bus.try_recv()
            {
                assert_eq!(
                    rx.try_recv(),
                    Some(7),
                    "wake delivered but the message is not queued"
                );
            }
            assert_eq!(producer.join().unwrap(), Ok(()));
        });
    }

    #[test]
    fn the_bound_holds_and_every_accepted_message_is_received_once() {
        loom::model(|| {
            let bus = WakeBus::new();
            let (shared, rx) = bounded_channel::<u32>(1);
            let tx = bounded_sender(shared, bus.clone(), TaskId::new(2, 0));
            let producer = loom::thread::spawn(move || [tx.try_send(1), tx.try_send(2)]);
            let mut got = Vec::new();
            got.extend(rx.try_recv());
            let sent = producer.join().unwrap();
            assert!(rx.len() <= 1, "more queued than the capacity");
            got.extend(rx.drain());
            let accepted: Vec<u32> = [1, 2]
                .into_iter()
                .zip(sent)
                .filter_map(|(m, r)| r.is_ok().then_some(m))
                .collect();
            assert_eq!(
                got, accepted,
                "accepted messages are received once, in order"
            );
            assert_eq!(
                sent[0],
                Ok(()),
                "the first send always finds the place free"
            );
            if let Err(e) = sent[1] {
                assert_eq!(e, TrySendError::Full(2));
            }
        });
    }
}
