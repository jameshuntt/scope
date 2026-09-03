//! Typed mailboxes: deliver a message *into* a running scoped task.
//!
//! waker_protocol's `Envelope` dispatched a boxed message into a `*mut ()` task
//! pointer — unsafe, and it required `'static` tasks. Scope does it safely: a
//! mailbox is just an `Arc`-shared typed queue plus a wake through the existing
//! [`WakeBus`]. The scheduler never has to store heterogeneous message types —
//! the queue lives beside the task, not in the slab — so this stays fully
//! `#![deny(unsafe_code)]` and works with scoped, non-`'static` messages.
//!
//! ```ignore
//! let (id, tx) = scope.spawn_with_mailbox("sink", |rx| Summer { rx, total: 0 });
//! tx.send(10);            // from anywhere holding the sender (incl. other threads)
//! tx.send(32);
//! scope.run();
//! ```

use crate::ids::TaskId;
use crate::sync::{Arc, Mutex};
use crate::wake::{WakeBus, WakeReason};
use std::collections::VecDeque;

/// The read end, held *inside* the receiving task. Drain it each poll.
pub struct Mailbox<M> {
    queue: Arc<Mutex<VecDeque<M>>>,
}

impl<M> Mailbox<M> {
    /// Pop the next message, if any.
    pub fn try_recv(&self) -> Option<M> {
        self.queue.lock().unwrap().pop_front()
    }

    /// Drain every currently-queued message.
    pub fn drain(&self) -> Vec<M> {
        let mut q = self.queue.lock().unwrap();
        q.drain(..).collect()
    }

    /// Whether nothing is queued.
    pub fn is_empty(&self) -> bool {
        self.queue.lock().unwrap().is_empty()
    }

    /// Number of queued messages.
    pub fn len(&self) -> usize {
        self.queue.lock().unwrap().len()
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
        self.queue.lock().unwrap().push_back(msg);
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
