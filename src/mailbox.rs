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
//!
//! A sender refused `Full` can wait for a place: [`BoundedSender::wake_when_free`] registers the
//! task in the mailbox, in arrival order. A receive (or drain) that frees places holds each one
//! for the oldest waiter before waking it with [`WakeReason::Capacity`], one waiter per place;
//! only that waiter can fill its place ([`BoundedSender::try_send_from`]), and every place nobody
//! holds stays open to every sender. The [`CapacityWait`] a registration returns gives the
//! registration and its held place up when it is dropped, so a waiter that ends without sending
//! hands its place to the next one.

use crate::ids::TaskId;
use crate::sync::{lock, Arc, Mutex};
use crate::wake::{WakeBus, WakeReason};
use std::collections::{BTreeMap, VecDeque};
use std::fmt;

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

/// The tasks waiting for a place, oldest registration first, one entry per task slot. Each
/// registration has a number, so a [`CapacityWait`] gives up only its own.
struct Waiters {
    /// Registration number -> task, in registration order.
    order: BTreeMap<u64, TaskId>,
    /// Task slot -> its registration number and task.
    by_slot: BTreeMap<u32, (u64, TaskId)>,
}

impl Waiters {
    const fn new() -> Self {
        Self {
            order: BTreeMap::new(),
            by_slot: BTreeMap::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    fn contains(&self, id: TaskId) -> bool {
        self.by_slot.get(&id.slot).is_some_and(|(_, t)| *t == id)
    }

    /// Adds `id` at the back as registration `seq`.
    fn push(&mut self, id: TaskId, seq: u64) {
        self.order.insert(seq, id);
        self.by_slot.insert(id.slot, (seq, id));
    }

    /// Removes the entry of `slot` whose task is not `id`: an older generation, whose task is
    /// gone. Whether there was one.
    fn drop_stale(&mut self, id: TaskId) -> bool {
        match self.by_slot.get(&id.slot) {
            Some((seq, t)) if *t != id => {
                let seq = *seq;
                self.by_slot.remove(&id.slot);
                self.order.remove(&seq);
                true
            }
            _ => false,
        }
    }

    /// Removes registration `seq` of `id`; whether it was there.
    fn remove(&mut self, id: TaskId, seq: u64) -> bool {
        match self.by_slot.get(&id.slot) {
            Some((s, t)) if *t == id && *s == seq => {
                self.by_slot.remove(&id.slot);
                self.order.remove(&seq);
                true
            }
            _ => false,
        }
    }

    /// Takes the oldest entry.
    fn pop(&mut self) -> Option<(TaskId, u64)> {
        let (seq, id) = self.order.pop_first()?;
        self.by_slot.remove(&id.slot);
        Some((id, seq))
    }

    /// Takes every entry, oldest first.
    fn take_all(&mut self) -> Vec<TaskId> {
        self.by_slot.clear();
        std::mem::take(&mut self.order).into_values().collect()
    }
}

/// What a bounded mailbox's two ends, and every [`CapacityWait`], share.
pub(crate) struct Bounded<M> {
    msgs: VecDeque<M>,
    capacity: usize,
    closed: bool,
    /// Tasks waiting for a place, oldest first.
    waiters: Waiters,
    /// Places held for woken waiters (task, registration), until each sends through
    /// [`BoundedSender::try_send_from`] or gives its place up. Nobody else can take them.
    held: Vec<(TaskId, u64)>,
    /// The next registration number.
    next: u64,
    /// The receiving task's scope bus, through which waiters are woken; set by the first sender.
    bus: Option<WakeBus>,
}

/// The waiters one operation woke, and the bus to wake them through once the lock is let go.
/// One place freed wakes one waiter, without allocating.
enum Woken {
    One(WakeBus, TaskId),
    Many(WakeBus, Vec<TaskId>),
}

impl Woken {
    /// Sends every wake, `WakeReason::Capacity`, oldest registration first.
    fn send(self) {
        match self {
            Woken::One(bus, id) => bus.wake(id, WakeReason::Capacity),
            Woken::Many(bus, ids) => {
                for id in ids {
                    bus.wake(id, WakeReason::Capacity);
                }
            }
        }
    }
}

fn send_wakes(w: Option<Woken>) {
    if let Some(w) = w {
        w.send();
    }
}

impl<M> Bounded<M> {
    /// Places nobody holds.
    fn free(&self) -> usize {
        if self.closed {
            0
        } else {
            self.capacity
                .saturating_sub(self.msgs.len() + self.held.len())
        }
    }

    /// Where in `held` a place of `id` is.
    fn held_by(&self, id: TaskId) -> Option<usize> {
        self.held.iter().position(|(t, _)| *t == id)
    }

    /// Places `id` may fill: the free ones and the one held for it.
    fn free_for(&self, id: TaskId) -> usize {
        if self.closed {
            0
        } else {
            self.free() + usize::from(self.held_by(id).is_some())
        }
    }

    /// Holds every free place for the oldest waiters, one each, before any of them is woken.
    /// A waiter is registered only while no place is free for it, so this holds at most as many
    /// places as the operation that calls it freed.
    fn hand_out(&mut self) -> Option<Woken> {
        if self.waiters.is_empty() || self.free() == 0 {
            return None;
        }
        let bus = self.bus.clone()?;
        let (id, seq) = self.waiters.pop()?;
        self.held.push((id, seq));
        if self.waiters.is_empty() || self.free() == 0 {
            return Some(Woken::One(bus, id));
        }
        let mut ids = vec![id];
        while self.free() > 0 {
            let Some((id, seq)) = self.waiters.pop() else {
                break;
            };
            self.held.push((id, seq));
            ids.push(id);
        }
        Some(Woken::Many(bus, ids))
    }

    /// Registration `seq` of `id` is given up: its entry leaves the table, and a place held for
    /// it goes to the next waiter.
    fn give_up(&mut self, id: TaskId, seq: u64) -> Option<Woken> {
        self.waiters.remove(id, seq);
        let i = self.held.iter().position(|h| *h == (id, seq))?;
        self.held.swap_remove(i);
        self.hand_out()
    }

    /// Closes: every held place is let go, every waiter is taken to be woken, oldest first.
    fn close(&mut self) -> Option<Woken> {
        self.closed = true;
        self.held.clear();
        if self.waiters.is_empty() {
            return None;
        }
        let bus = self.bus.clone()?;
        Some(Woken::Many(bus, self.waiters.take_all()))
    }
}

/// The read end of a bounded mailbox, held *inside* the receiving task. Dropping it (the task
/// finishing, by any path) closes the mailbox and drops what is still queued.
pub struct BoundedMailbox<M> {
    shared: Arc<Mutex<Bounded<M>>>,
}

impl<M> BoundedMailbox<M> {
    /// Pop the next message, if any. Frees one place: when a task waits for one
    /// ([`BoundedSender::wake_when_free`]), the place is held for the oldest waiter and it is woken
    /// (`WakeReason::Capacity`).
    pub fn try_recv(&self) -> Option<M> {
        let (msg, woken) = {
            let mut q = lock(&self.shared);
            let msg = q.msgs.pop_front();
            let woken = if msg.is_some() { q.hand_out() } else { None };
            (msg, woken)
        };
        send_wakes(woken);
        msg
    }

    /// Drain every currently-queued message. Frees every place: as many waiters as places were
    /// freed get one held each and are woken, oldest first.
    pub fn drain(&self) -> Vec<M> {
        let (out, woken) = {
            let mut q = lock(&self.shared);
            let out: Vec<M> = q.msgs.drain(..).collect();
            let woken = if out.is_empty() { None } else { q.hand_out() };
            (out, woken)
        };
        send_wakes(woken);
        out
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

    /// Refuse every later send with `Closed`. Messages already queued stay readable. Every
    /// waiting sender is woken (`WakeReason::Capacity`; it will see `Closed`) and every held
    /// place is let go.
    pub fn close(&self) {
        let woken = lock(&self.shared).close();
        send_wakes(woken);
    }
}

impl<M> Drop for BoundedMailbox<M> {
    fn drop(&mut self) {
        // Take the messages out under the lock and drop them after it.
        let (rest, woken) = {
            let mut q = lock(&self.shared);
            let woken = q.close();
            (std::mem::take(&mut q.msgs), woken)
        };
        drop(rest);
        send_wakes(woken);
    }
}

/// What [`BoundedSender::wake_when_free`] did.
#[derive(Debug)]
#[must_use = "a Registered wait is given up when its CapacityWait is dropped"]
pub enum WaitRegistration<M> {
    /// The task waits in the mailbox: one `WakeReason::Capacity` wake comes when a place is held
    /// for it, or when the mailbox closes. Keep the [`CapacityWait`]: dropping it gives the
    /// registration, and the place once held, up.
    Registered(CapacityWait<M>),
    /// The task already waits: it keeps its turn, and the first registration's
    /// [`CapacityWait`] still stands for it.
    AlreadyRegistered,
    /// A place is free for the task now (a free one, or the one held for it since its wake):
    /// nothing was registered; send again.
    FreeNow,
    /// The mailbox is closed: nothing was registered.
    Closed,
}

/// One registration of a task waiting for a place in a bounded mailbox, from
/// [`BoundedSender::wake_when_free`]. While it lives the task keeps its turn and, once woken, the
/// place held for it. Dropping it (or [`CapacityWait::cancel`]) gives both up: the entry leaves
/// the mailbox's table, and a place held for the task goes to the next waiter, who is woken.
///
/// Keep it in the waiting task's own state. Scope drops a task's state when the task ends by any
/// path (it completes, is cancelled or panics), so a place held for a task that ends without
/// sending goes on to the next waiter instead of being lost. A `CapacityWait` kept anywhere else
/// keeps its place until it is dropped.
#[must_use = "dropping a CapacityWait gives the registration up at once"]
pub struct CapacityWait<M> {
    shared: Arc<Mutex<Bounded<M>>>,
    waiter: TaskId,
    seq: u64,
}

impl<M> CapacityWait<M> {
    /// The waiting task.
    pub fn waiter(&self) -> TaskId {
        self.waiter
    }

    /// Gives the registration up now (the same as dropping it).
    pub fn cancel(self) {}
}

impl<M> fmt::Debug for CapacityWait<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CapacityWait")
            .field("waiter", &self.waiter)
            .field("registration", &self.seq)
            .finish()
    }
}

impl<M> Drop for CapacityWait<M> {
    fn drop(&mut self) {
        let woken = lock(&self.shared).give_up(self.waiter, self.seq);
        send_wakes(woken);
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
    /// `Full` when the mailbox holds its capacity (a place held for a woken waiter counts as
    /// taken), `Closed` when the receiver is gone. A refused message enqueues nothing, wakes
    /// nothing and allocates nothing.
    pub fn try_send(&self, msg: M) -> Result<(), TrySendError<M>> {
        {
            let mut q = lock(&self.shared);
            if q.closed {
                return Err(TrySendError::Closed(msg));
            }
            if q.free() == 0 {
                return Err(TrySendError::Full(msg));
            }
            q.msgs.push_back(msg);
        }
        self.bus.wake(self.target, WakeReason::Inbox);
        Ok(())
    }

    /// [`BoundedSender::try_send`] by task `from`: a place held for `from` (it was woken for it,
    /// see [`BoundedSender::wake_when_free`]) is its to fill, and filling it uses the hold up;
    /// otherwise the same as `try_send`.
    pub fn try_send_from(&self, msg: M, from: TaskId) -> Result<(), TrySendError<M>> {
        {
            let mut q = lock(&self.shared);
            if q.closed {
                return Err(TrySendError::Closed(msg));
            }
            match q.held_by(from) {
                Some(i) => {
                    q.held.swap_remove(i);
                }
                None if q.free() == 0 => return Err(TrySendError::Full(msg)),
                None => {}
            }
            q.msgs.push_back(msg);
        }
        self.bus.wake(self.target, WakeReason::Inbox);
        Ok(())
    }

    /// Wake task `waiter` (of the receiving task's scope) once, with [`WakeReason::Capacity`],
    /// when a place is held for it, or when the mailbox closes. Checked under the mailbox's lock,
    /// so a place freed after a refused send is never missed: with a place free for `waiter` now,
    /// nothing is registered (`FreeNow`, send again).
    ///
    /// Waiters are served in the order they registered: when places are freed (a receive, a
    /// drain, a waiter giving its place up), each is held for the oldest waiter, one place each,
    /// before any of them is woken, and only that waiter can fill it (through
    /// [`BoundedSender::try_send_from`]); senders that do not wait still use every place nobody
    /// holds. A task has at most one registration per mailbox: a second call keeps its turn
    /// (`AlreadyRegistered`). A registration of a slot's newer generation drops the older one's
    /// entry and held place, whose task is gone, so a stale id never takes a place from its
    /// slot's next task. The [`CapacityWait`] in `Registered` gives the registration up when it
    /// is dropped.
    pub fn wake_when_free(&self, waiter: TaskId) -> WaitRegistration<M> {
        let (reg, woken) = {
            let mut q = lock(&self.shared);
            if q.closed {
                return WaitRegistration::Closed;
            }
            // The slot's older generation is gone: its entry and its held place go.
            q.waiters.drop_stale(waiter);
            let stale = q
                .held
                .iter()
                .position(|(t, _)| t.slot == waiter.slot && *t != waiter);
            let woken = match stale {
                Some(i) => {
                    q.held.swap_remove(i);
                    q.hand_out()
                }
                None => None,
            };
            let reg = if q.free_for(waiter) > 0 {
                WaitRegistration::FreeNow
            } else if q.waiters.contains(waiter) {
                WaitRegistration::AlreadyRegistered
            } else {
                if q.bus.is_none() {
                    q.bus = Some(self.bus.clone());
                }
                let seq = q.next;
                q.next += 1;
                q.waiters.push(waiter, seq);
                WaitRegistration::Registered(CapacityWait {
                    shared: self.shared.clone(),
                    waiter,
                    seq,
                })
            };
            (reg, woken)
        };
        send_wakes(woken);
        reg
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

    /// Places left: how many `try_send`s would succeed now if nothing else sent or received. A
    /// place held for a woken waiter is not free. Zero once the mailbox is closed.
    pub fn free(&self) -> usize {
        lock(&self.shared).free()
    }

    /// Places task `waiter` may fill now: [`BoundedSender::free`], plus the place held for it
    /// when it was woken for one. Zero once the mailbox is closed.
    pub fn free_for(&self, waiter: TaskId) -> usize {
        lock(&self.shared).free_for(waiter)
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
        waiters: Waiters::new(),
        held: Vec::new(),
        next: 0,
        bus: None,
    }));
    (shared.clone(), BoundedMailbox { shared })
}

pub(crate) fn bounded_sender<M>(
    shared: Arc<Mutex<Bounded<M>>>,
    bus: WakeBus,
    target: TaskId,
) -> BoundedSender<M> {
    {
        let mut q = lock(&shared);
        if q.bus.is_none() {
            q.bus = Some(bus.clone());
        }
    }
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

/// Capacity waiters: the table, the held places and the wakes, against a bare bus.
#[cfg(all(test, not(loom)))]
mod capacity_wait_tests {
    use super::*;
    use crate::wake::SchedulerMsg;

    fn pair(capacity: usize) -> (BoundedSender<u32>, BoundedMailbox<u32>, WakeBus) {
        let bus = WakeBus::new();
        let (shared, rx) = bounded_channel(capacity);
        (
            bounded_sender(shared, bus.clone(), TaskId::new(0, 0)),
            rx,
            bus,
        )
    }

    /// Every wake on the bus, in order.
    fn wakes(bus: &WakeBus) -> Vec<(TaskId, WakeReason)> {
        let mut out = Vec::new();
        while let Some(SchedulerMsg::Wake { id, reason }) = bus.try_recv() {
            out.push((id, reason));
        }
        out
    }

    /// The `Capacity` wakes on the bus, in order (the `Inbox` wakes of accepted sends dropped).
    fn capacity_wakes(bus: &WakeBus) -> Vec<TaskId> {
        wakes(bus)
            .into_iter()
            .filter(|(_, r)| *r == WakeReason::Capacity)
            .map(|(id, _)| id)
            .collect()
    }

    fn fill(tx: &BoundedSender<u32>, n: usize) {
        for i in 0..n {
            assert_eq!(tx.try_send(i as u32), Ok(()));
        }
    }

    fn t(slot: u32) -> TaskId {
        TaskId::new(slot, 0)
    }

    fn registered(r: WaitRegistration<u32>) -> CapacityWait<u32> {
        match r {
            WaitRegistration::Registered(w) => w,
            other => panic!("not registered: {other:?}"),
        }
    }

    #[test]
    fn one_freed_place_wakes_one_waiter() {
        let (tx, rx, bus) = pair(1);
        fill(&tx, 1);
        let _a = registered(tx.wake_when_free(t(1)));
        let _b = registered(tx.wake_when_free(t(2)));
        wakes(&bus);
        assert_eq!(rx.try_recv(), Some(0));
        assert_eq!(wakes(&bus), [(t(1), WakeReason::Capacity)]);
        assert_eq!(rx.try_recv(), None, "an empty receive frees nothing");
        assert!(wakes(&bus).is_empty());
    }

    #[test]
    fn n_freed_places_wake_n_waiters() {
        let (tx, rx, bus) = pair(4);
        fill(&tx, 4);
        let ws: Vec<_> = (1..=6)
            .map(|s| registered(tx.wake_when_free(t(s))))
            .collect();
        wakes(&bus);
        assert_eq!(rx.drain().len(), 4);
        assert_eq!(capacity_wakes(&bus), [t(1), t(2), t(3), t(4)]);
        assert_eq!(tx.free(), 0, "the four places are held for the four woken");
        drop(ws);
    }

    #[test]
    fn waiters_are_served_in_registration_order() {
        let (tx, rx, bus) = pair(1);
        fill(&tx, 1);
        // registered in the order 3, 1, 4, 2: served in that order, not by slot
        let ws: Vec<_> = [3, 1, 4, 2]
            .into_iter()
            .map(|s| registered(tx.wake_when_free(t(s))))
            .collect();
        wakes(&bus);
        let mut got = Vec::new();
        for _ in 0..4 {
            assert!(rx.try_recv().is_some());
            let w = capacity_wakes(&bus);
            assert_eq!(w.len(), 1);
            got.push(w[0].slot);
            assert_eq!(tx.try_send_from(9, w[0]), Ok(()));
        }
        assert_eq!(got, [3, 1, 4, 2]);
        drop(ws);
    }

    #[test]
    fn a_held_place_cannot_be_stolen_and_the_rest_are_free_to_all() {
        let (tx, rx, bus) = pair(2);
        fill(&tx, 2);
        let a = registered(tx.wake_when_free(t(1)));
        wakes(&bus);
        assert_eq!(rx.try_recv(), Some(0));
        assert_eq!(capacity_wakes(&bus), [t(1)]);
        assert_eq!((tx.free(), tx.free_for(t(1)), tx.free_for(t(2))), (0, 1, 0));
        assert_eq!(
            tx.try_send(7),
            Err(TrySendError::Full(7)),
            "a sender that does not wait"
        );
        assert_eq!(
            tx.try_send_from(8, t(2)),
            Err(TrySendError::Full(8)),
            "another task"
        );
        // a second place freed with nobody waiting is free to every sender
        assert_eq!(rx.try_recv(), Some(1));
        assert!(capacity_wakes(&bus).is_empty());
        assert_eq!((tx.free(), tx.free_for(t(1))), (1, 2));
        assert_eq!(tx.try_send(5), Ok(()));
        assert_eq!(tx.try_send(6), Err(TrySendError::Full(6)));
        // the woken waiter's send uses its own place up
        assert_eq!(tx.try_send_from(9, t(1)), Ok(()));
        assert_eq!(tx.free_for(t(1)), 0);
        assert_eq!(rx.drain(), [5, 9]);
        drop(a);
        assert_eq!(tx.free(), 2, "dropping a used registration changes nothing");
    }

    #[test]
    fn a_second_registration_keeps_the_first_turn() {
        let (tx, rx, bus) = pair(1);
        fill(&tx, 1);
        let a = registered(tx.wake_when_free(t(1)));
        let b = registered(tx.wake_when_free(t(2)));
        assert!(matches!(
            tx.wake_when_free(t(1)),
            WaitRegistration::AlreadyRegistered
        ));
        assert!(matches!(
            tx.wake_when_free(t(1)),
            WaitRegistration::AlreadyRegistered
        ));
        wakes(&bus);
        rx.try_recv();
        assert_eq!(capacity_wakes(&bus), [t(1)], "still first");
        // woken, with its place held: a place is free for it now
        assert!(matches!(tx.wake_when_free(t(1)), WaitRegistration::FreeNow));
        drop((a, b));
    }

    #[test]
    fn cancelling_before_the_wake_leaves_the_table() {
        let (tx, rx, bus) = pair(1);
        fill(&tx, 1);
        let a = registered(tx.wake_when_free(t(1)));
        let b = registered(tx.wake_when_free(t(2)));
        a.cancel();
        wakes(&bus);
        rx.try_recv();
        assert_eq!(
            capacity_wakes(&bus),
            [t(2)],
            "the cancelled waiter is not woken"
        );
        drop(b);
        // with nobody waiting the place was given back
        assert_eq!(tx.free(), 1);
    }

    #[test]
    fn cancelling_after_the_wake_hands_the_held_place_on() {
        let (tx, rx, bus) = pair(1);
        fill(&tx, 1);
        let a = registered(tx.wake_when_free(t(1)));
        let b = registered(tx.wake_when_free(t(2)));
        let c = registered(tx.wake_when_free(t(3)));
        wakes(&bus);
        rx.try_recv();
        assert_eq!(capacity_wakes(&bus), [t(1)]);
        drop(a); // woken, never sent: its place goes to the next waiter
        assert_eq!(capacity_wakes(&bus), [t(2)]);
        assert_eq!((tx.free_for(t(1)), tx.free_for(t(2))), (0, 1));
        drop(b); // and on again
        assert_eq!(capacity_wakes(&bus), [t(3)]);
        drop(c); // the last waiter leaves: the place is free to everyone
        assert!(capacity_wakes(&bus).is_empty());
        assert_eq!(tx.free(), 1);
        assert_eq!(tx.try_send(4), Ok(()));
    }

    #[test]
    fn a_registration_dropped_after_its_send_does_not_touch_a_newer_one() {
        let (tx, rx, bus) = pair(1);
        fill(&tx, 1);
        let first = registered(tx.wake_when_free(t(1)));
        rx.try_recv();
        assert_eq!(tx.try_send_from(1, t(1)), Ok(()));
        let second = registered(tx.wake_when_free(t(1)));
        drop(first); // the old registration: its place was used, the new one stays
        wakes(&bus);
        rx.try_recv();
        assert_eq!(capacity_wakes(&bus), [t(1)]);
        drop(second);
    }

    #[test]
    fn close_and_drop_wake_every_waiter_and_registration_after_is_closed() {
        for close in [true, false] {
            let (tx, rx, bus) = pair(1);
            fill(&tx, 1);
            let ws: Vec<_> = (1..=3)
                .map(|s| registered(tx.wake_when_free(t(s))))
                .collect();
            wakes(&bus);
            if close {
                rx.close();
            } else {
                drop(rx);
            }
            assert_eq!(capacity_wakes(&bus), [t(1), t(2), t(3)]);
            assert!(matches!(tx.wake_when_free(t(4)), WaitRegistration::Closed));
            assert_eq!(tx.try_send(1), Err(TrySendError::Closed(1)));
            assert_eq!(tx.try_send_from(1, t(1)), Err(TrySendError::Closed(1)));
            assert_eq!(tx.free_for(t(1)), 0);
            drop(ws); // giving up after the close wakes nobody
            assert!(wakes(&bus).is_empty());
        }
    }

    #[test]
    fn close_lets_held_places_go() {
        let (tx, rx, bus) = pair(1);
        fill(&tx, 1);
        let a = registered(tx.wake_when_free(t(1)));
        let _b = registered(tx.wake_when_free(t(2)));
        rx.try_recv();
        wakes(&bus);
        rx.close();
        assert_eq!(
            capacity_wakes(&bus),
            [t(2)],
            "the waiter is woken; the held place is gone"
        );
        assert_eq!(tx.try_send_from(1, t(1)), Err(TrySendError::Closed(1)));
        drop(a);
        assert!(wakes(&bus).is_empty());
    }

    #[test]
    fn a_stale_generation_never_takes_its_slots_next_task_place() {
        let (tx, rx, bus) = pair(1);
        fill(&tx, 1);
        let (old, new) = (TaskId::new(5, 0), TaskId::new(5, 1));
        let o = registered(tx.wake_when_free(old));
        let other = registered(tx.wake_when_free(t(6)));
        // the slot is reused: the newer generation registers and the older entry goes
        let n = registered(tx.wake_when_free(new));
        wakes(&bus);
        rx.try_recv();
        assert_eq!(capacity_wakes(&bus), [t(6)]);
        assert_eq!(tx.try_send_from(1, t(6)), Ok(()));
        rx.try_recv();
        assert_eq!(capacity_wakes(&bus), [new], "never the older generation");
        drop(o); // the stale registration's guard: nothing of the newer one changes
        assert_eq!(tx.free_for(new), 1);
        drop((other, n));
    }

    #[test]
    fn a_stale_generations_held_place_goes_on_when_the_slots_next_task_registers() {
        let (tx, rx, bus) = pair(1);
        fill(&tx, 1);
        let (old, new) = (TaskId::new(5, 0), TaskId::new(5, 1));
        let o = registered(tx.wake_when_free(old));
        let other = registered(tx.wake_when_free(t(6)));
        rx.try_recv();
        assert_eq!(capacity_wakes(&bus), [old]);
        // the old task is gone but its CapacityWait was kept elsewhere; its slot's next task
        // registers
        let n = tx.wake_when_free(new);
        assert_eq!(
            capacity_wakes(&bus),
            [t(6)],
            "the stale place went to the oldest waiter"
        );
        assert!(matches!(n, WaitRegistration::Registered(_)));
        assert_eq!((tx.free_for(old), tx.free_for(t(6))), (0, 1));
        drop(o); // nothing of the old registration is left to give up
        assert_eq!((tx.free_for(t(6)), tx.free()), (1, 0));
        drop((other, n));
    }

    #[test]
    #[cfg_attr(miri, ignore = "four threads spinning on a lock: minutes under Miri")]
    fn concurrent_senders_keep_the_bound_and_lose_no_place() {
        // four sender threads, each a task that sends 25 messages and waits for a place when
        // the mailbox is full (spinning on free_for until a place is held for it); the main
        // thread receives. Every message arrives once and the bound holds; a lost wake or a lost
        // place would leave a sender spinning and the test hanging.
        let (tx, rx, _bus) = pair(2);
        let received = std::thread::scope(|s| {
            for k in 1..=4u32 {
                let tx = tx.clone();
                s.spawn(move || {
                    let me = t(k);
                    let mut wait = None;
                    for m in 0..25u32 {
                        loop {
                            match tx.try_send_from(k * 100 + m, me) {
                                Ok(()) => {
                                    // the place held for it is used up: the registration is spent
                                    drop(wait.take());
                                    break;
                                }
                                Err(TrySendError::Closed(_)) => panic!("closed"),
                                Err(TrySendError::Full(_)) => match tx.wake_when_free(me) {
                                    WaitRegistration::FreeNow => {}
                                    WaitRegistration::Registered(w) => {
                                        wait = Some(w);
                                        while tx.free_for(me) == 0 {
                                            std::thread::yield_now();
                                        }
                                    }
                                    WaitRegistration::AlreadyRegistered => std::thread::yield_now(),
                                    WaitRegistration::Closed => panic!("closed"),
                                },
                            }
                        }
                    }
                    drop(wait);
                });
            }
            let mut got = Vec::new();
            while got.len() < 100 {
                assert!(rx.len() <= 2, "more queued than the capacity");
                match rx.try_recv() {
                    Some(m) => got.push(m),
                    None => std::thread::yield_now(),
                }
            }
            got
        });
        let mut sorted = received.clone();
        sorted.sort_unstable();
        let want: Vec<u32> = (1..=4)
            .flat_map(|k| (0..25).map(move |m| k * 100 + m))
            .collect();
        assert_eq!(sorted, want, "every message once");
        for k in 1..=4u32 {
            let mine: Vec<u32> = received.iter().copied().filter(|m| m / 100 == k).collect();
            assert!(
                mine.windows(2).all(|w| w[0] < w[1]),
                "each sender's order kept"
            );
        }
        assert_eq!(tx.free(), 2, "nothing held once every sender is done");
    }

    #[test]
    fn a_drain_frees_many_and_wakes_as_many_and_no_more() {
        let (tx, rx, bus) = pair(4);
        fill(&tx, 3);
        assert!(matches!(tx.wake_when_free(t(9)), WaitRegistration::FreeNow));
        fill(&tx, 1);
        let ws: Vec<_> = (1..=6)
            .map(|s| registered(tx.wake_when_free(t(s))))
            .collect();
        wakes(&bus);
        assert_eq!(rx.drain().len(), 4);
        assert_eq!(capacity_wakes(&bus), [t(1), t(2), t(3), t(4)]);
        assert_eq!(rx.drain().len(), 0);
        assert!(
            capacity_wakes(&bus).is_empty(),
            "an empty drain frees nothing"
        );
        drop(ws);
    }

    #[test]
    fn a_zero_capacity_mailbox_registers_and_only_its_close_wakes() {
        let (tx, rx, bus) = pair(0);
        assert_eq!(tx.try_send(1), Err(TrySendError::Full(1)));
        let w = registered(tx.wake_when_free(t(1)));
        assert_eq!(tx.free_for(t(1)), 0);
        assert_eq!(rx.try_recv(), None);
        assert_eq!(rx.drain(), Vec::<u32>::new());
        assert!(wakes(&bus).is_empty());
        rx.close();
        assert_eq!(capacity_wakes(&bus), [t(1)]);
        drop(w);
    }

    #[test]
    fn without_waiters_bounded_sends_are_as_before() {
        // 0.3.2's behaviour, waiters never registered: the same results and the same wakes
        let (tx, rx, bus) = pair(2);
        assert_eq!((tx.free(), tx.free_for(t(1))), (2, 2));
        assert_eq!(tx.try_send(1), Ok(()));
        assert_eq!(tx.try_send_from(2, t(1)), Ok(()));
        assert_eq!(tx.try_send(3), Err(TrySendError::Full(3)));
        assert_eq!(rx.try_recv(), Some(1));
        assert_eq!(tx.free(), 1);
        assert_eq!(rx.drain(), [2]);
        assert_eq!(
            wakes(&bus),
            [
                (TaskId::new(0, 0), WakeReason::Inbox),
                (TaskId::new(0, 0), WakeReason::Inbox)
            ]
        );
    }
}

/// Loom models of capacity waiters: a registration, a freed place, a racing sender and a
/// waiter giving up, in every interleaving. Capacity one, two or three threads.
///
/// Run:  RUSTFLAGS="--cfg loom" cargo test --lib --release
#[cfg(all(test, loom))]
mod loom_capacity_waiters {
    use super::*;
    use crate::wake::SchedulerMsg;

    fn capacity_wakes(bus: &WakeBus) -> Vec<TaskId> {
        let mut out = Vec::new();
        while let Some(SchedulerMsg::Wake { id, reason }) = bus.try_recv() {
            if reason == WakeReason::Capacity {
                out.push(id);
            }
        }
        out
    }

    /// The waiter's side: send as `me`; on `Full` register, and send again while a place is
    /// free for it. `Ok(None)`: sent. `Ok(Some(wait))`: registered, not sent.
    fn send_or_wait(
        tx: &BoundedSender<u32>,
        me: TaskId,
        msg: u32,
    ) -> Result<Option<CapacityWait<u32>>, ()> {
        loop {
            match tx.try_send_from(msg, me) {
                Ok(()) => return Ok(None),
                Err(TrySendError::Closed(_)) => return Err(()),
                Err(TrySendError::Full(_)) => match tx.wake_when_free(me) {
                    WaitRegistration::FreeNow => continue,
                    WaitRegistration::Registered(w) => return Ok(Some(w)),
                    WaitRegistration::AlreadyRegistered | WaitRegistration::Closed => {
                        return Err(())
                    }
                },
            }
        }
    }

    /// A waiter and a receive race, with a sender that does not wait racing both: the bound
    /// holds, nothing is lost or taken twice, and a registered waiter is never left without a
    /// wake while a place is held for it or free: either it was woken and the place is its, or
    /// no place came free after it registered.
    #[test]
    fn register_free_and_a_racing_send_lose_no_wake_and_no_place() {
        loom::model(|| {
            let bus = WakeBus::new();
            let (shared, rx) = bounded_channel::<u32>(1);
            let tx = bounded_sender(shared, bus.clone(), TaskId::new(0, 0));
            assert_eq!(tx.try_send(100), Ok(()));
            let me = TaskId::new(1, 0);
            let waiter = {
                let tx = tx.clone();
                loom::thread::spawn(move || send_or_wait(&tx, me, 1))
            };
            let racer = {
                let tx = tx.clone();
                loom::thread::spawn(move || tx.try_send(2))
            };
            let first = rx.try_recv();
            let waited = waiter.join().unwrap().expect("never closed");
            let raced = racer.join().unwrap();
            assert_eq!(
                first,
                Some(100),
                "the first place's message comes out first"
            );
            assert!(rx.len() <= 1, "more queued than the capacity");
            let woken = capacity_wakes(&bus);
            let sent = usize::from(waited.is_none()) + usize::from(raced.is_ok());
            assert!(sent <= 1, "one place freed, at most one more accepted");
            match &waited {
                Some(_) => {
                    // registered: woken with the place held for it, or no place came free
                    // after it registered (the racer took it, or the receive came first and
                    // the waiter's send then failed because the racer filled it)
                    if woken == [me] {
                        assert_eq!(tx.free_for(me), 1, "woken, so the place is its");
                        assert!(raced.is_err(), "nobody else took the held place");
                    } else {
                        assert!(woken.is_empty());
                        assert_eq!(
                            tx.free_for(me),
                            0,
                            "lost wake: a place is free for a waiter nobody woke"
                        );
                    }
                }
                None => assert!(
                    woken.is_empty(),
                    "a waiter that sent was never registered and woken"
                ),
            }
            let mut got: Vec<u32> = first.into_iter().collect();
            got.extend(rx.drain());
            assert_eq!(
                got.len(),
                1 + sent,
                "every accepted message is received once"
            );
            drop(waited);
        });
    }

    /// Two waiters race each other and a receive: whatever the order, no place is free while
    /// a registered waiter has none held (the wake that would have served it was not lost), at
    /// most one place is held, and it is held for a task that was woken for it.
    #[test]
    fn two_waiters_and_a_receive_never_leave_a_free_place_beside_a_waiter() {
        loom::model(|| {
            let bus = WakeBus::new();
            let (shared, rx) = bounded_channel::<u32>(1);
            let tx = bounded_sender(shared, bus.clone(), TaskId::new(0, 0));
            assert_eq!(tx.try_send(100), Ok(()));
            let (a, b) = (TaskId::new(1, 0), TaskId::new(2, 0));
            let ta = {
                let tx = tx.clone();
                loom::thread::spawn(move || send_or_wait(&tx, a, 1))
            };
            let tb = {
                let tx = tx.clone();
                loom::thread::spawn(move || send_or_wait(&tx, b, 2))
            };
            assert_eq!(rx.try_recv(), Some(100));
            let wa = ta.join().unwrap().expect("never closed");
            let wb = tb.join().unwrap().expect("never closed");
            let woken = capacity_wakes(&bus);
            let sent = usize::from(wa.is_none()) + usize::from(wb.is_none());
            assert!(sent <= 1, "one place freed, at most one more accepted");
            assert!(rx.len() <= 1);
            for (me, w) in [(a, &wa), (b, &wb)] {
                if w.is_some() && tx.free_for(me) == 0 {
                    assert_eq!(tx.free(), 0, "a place is free while {me} waits unwoken");
                }
                if tx.free_for(me) > tx.free() {
                    assert!(
                        woken.contains(&me),
                        "{me} holds a place it was not woken for"
                    );
                }
            }
            assert!(
                woken.len() <= 1,
                "one freed place, at most one wake: {woken:?}"
            );
            drop((wa, wb));
            assert_eq!(
                tx.free() + rx.len(),
                1,
                "every place is back once both are gone"
            );
        });
    }

    /// A woken waiter gives its place up while the receive frees another: the place goes to the
    /// next waiter, and nothing is held for a registration that is gone.
    #[test]
    fn giving_up_while_a_place_is_freed_hands_the_place_on() {
        loom::model(|| {
            let bus = WakeBus::new();
            let (shared, rx) = bounded_channel::<u32>(1);
            let tx = bounded_sender(shared, bus.clone(), TaskId::new(0, 0));
            assert_eq!(tx.try_send(1), Ok(()));
            let (a, b) = (TaskId::new(1, 0), TaskId::new(2, 0));
            let wa = match tx.wake_when_free(a) {
                WaitRegistration::Registered(w) => w,
                other => panic!("{other:?}"),
            };
            let wb = match tx.wake_when_free(b) {
                WaitRegistration::Registered(w) => w,
                other => panic!("{other:?}"),
            };
            let t = loom::thread::spawn(move || drop(wa));
            assert_eq!(rx.try_recv(), Some(1));
            t.join().unwrap();
            // a gave up (before or after its place was held): b holds the one place now
            assert_eq!(
                tx.free_for(a),
                0,
                "a place held for a registration that is gone"
            );
            assert_eq!(tx.free_for(b), 1, "the freed place went to the next waiter");
            assert_eq!(tx.free(), 0);
            let woken = capacity_wakes(&bus);
            assert!(woken.contains(&b), "b was woken for its place: {woken:?}");
            assert!(woken.len() <= 2);
            drop(wb);
            assert_eq!(tx.free(), 1, "the last waiter leaving frees the place");
        });
    }

    /// A close races a registration: the waiter is either refused (`Closed`) or registered and
    /// woken by the close, never registered and left asleep.
    #[test]
    fn a_close_racing_a_registration_never_leaves_a_waiter_asleep() {
        loom::model(|| {
            let bus = WakeBus::new();
            let (shared, rx) = bounded_channel::<u32>(1);
            let tx = bounded_sender(shared, bus.clone(), TaskId::new(0, 0));
            assert_eq!(tx.try_send(1), Ok(()));
            let me = TaskId::new(1, 0);
            let t = {
                let tx = tx.clone();
                loom::thread::spawn(move || tx.wake_when_free(me))
            };
            rx.close();
            let reg = t.join().unwrap();
            let woken = capacity_wakes(&bus);
            match reg {
                WaitRegistration::Registered(_) => assert_eq!(woken, [me], "the close woke it"),
                WaitRegistration::Closed => assert!(woken.is_empty()),
                other => panic!("{other:?}"),
            }
        });
    }
}
