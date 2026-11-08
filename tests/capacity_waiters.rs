//! Capacity waiters on a running scope: sender tasks that wait for a place in a bounded mailbox
//! through `BoundedSender::wake_when_free`, a receiver task that frees one place a poll, and what
//! becomes of a held place when its task ends without sending.

use std::cell::RefCell;
use std::rc::Rc;

use scope::{
    BoundedMailbox, BoundedSender, CapacityWait, Cx, SchedulerConfig, ScopedTask, TaskId, TaskPoll,
    Tracer, TrySendError, WaitRegistration, WakeOrdering, WakeReason,
};

/// What the receiver took, in order, and the senders' wakes.
#[derive(Default)]
struct Log {
    received: Vec<(u32, u32)>,
    refused: u32,
    capacity_wakes: Vec<TaskId>,
    left: Vec<u32>,
}

/// Sends `count` messages; when the mailbox is full it waits for a place. A `leaver` sends
/// nothing: it waits for a place and, once one is held for it, ends (its `CapacityWait` is
/// dropped with it).
struct Sender {
    who: u32,
    tx: BoundedSender<(u32, u32)>,
    next: u32,
    count: u32,
    wait: Option<CapacityWait<(u32, u32)>>,
    log: Rc<RefCell<Log>>,
    leaver: bool,
}

impl<'env, R> ScopedTask<'env, R> for Sender {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        let me = cx.id();
        if self.leaver {
            if self.wait.is_some() && self.tx.free_for(me) > 0 {
                self.log.borrow_mut().left.push(self.who);
                return TaskPoll::Ready;
            }
            return match self.tx.wake_when_free(me) {
                WaitRegistration::Registered(w) => {
                    self.wait = Some(w);
                    TaskPoll::Pending
                }
                WaitRegistration::AlreadyRegistered => TaskPoll::Pending,
                WaitRegistration::FreeNow => cx.yield_now(),
                WaitRegistration::Closed => TaskPoll::Ready,
            };
        }
        while self.next < self.count {
            match self.tx.try_send_from((self.who, self.next), me) {
                Ok(()) => {
                    self.next += 1;
                    self.wait = None;
                }
                Err(TrySendError::Closed(_)) => return TaskPoll::Ready,
                Err(TrySendError::Full(_)) => {
                    self.log.borrow_mut().refused += 1;
                    match self.tx.wake_when_free(me) {
                        WaitRegistration::FreeNow => {}
                        WaitRegistration::Registered(w) => {
                            self.wait = Some(w);
                            return TaskPoll::Pending;
                        }
                        WaitRegistration::AlreadyRegistered => return TaskPoll::Pending,
                        WaitRegistration::Closed => return TaskPoll::Ready,
                    }
                }
            }
        }
        TaskPoll::Ready
    }
}

/// Takes one message a poll until it has `expect`.
struct Receiver {
    rx: BoundedMailbox<(u32, u32)>,
    log: Rc<RefCell<Log>>,
    expect: usize,
}

impl<'env, R> ScopedTask<'env, R> for Receiver {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        if let Some(m) = self.rx.try_recv() {
            self.log.borrow_mut().received.push(m);
        }
        if self.log.borrow().received.len() >= self.expect {
            TaskPoll::Ready
        } else if self.rx.is_empty() {
            TaskPoll::Pending
        } else {
            cx.yield_now()
        }
    }
}

struct Wakes(Rc<RefCell<Log>>);

impl Tracer for Wakes {
    fn on_wake(&mut self, id: TaskId, reason: WakeReason) {
        if reason == WakeReason::Capacity {
            self.0.borrow_mut().capacity_wakes.push(id);
        }
    }
}

fn run(
    senders: u32,
    per_sender: u32,
    capacity: usize,
    leavers: &[u32],
    ordering: WakeOrdering,
) -> Log {
    let log = Rc::new(RefCell::new(Log::default()));
    let mut env = ();
    let mut scope = scope::Scope::with_config(&mut env, SchedulerConfig::with_ordering(ordering));
    scope.set_tracer(Box::new(Wakes(log.clone())));
    let expect = ((senders - leavers.len() as u32) * per_sender) as usize;
    let (_rid, tx) = scope.spawn_with_bounded_mailbox("receiver", capacity, |rx| Receiver {
        rx,
        log: log.clone(),
        expect,
    });
    for who in 0..senders {
        scope.spawn_named(
            format!("sender {who}"),
            Sender {
                who,
                tx: tx.clone(),
                next: 0,
                count: per_sender,
                wait: None,
                log: log.clone(),
                leaver: leavers.contains(&who),
            },
        );
    }
    let report = scope.run();
    assert!(report.all_completed(), "{report:?}");
    drop(scope);
    drop(tx);
    Rc::try_unwrap(log)
        .ok()
        .expect("the scope is gone")
        .into_inner()
}

#[test]
fn every_message_arrives_once_in_each_senders_order_and_each_wake_finds_its_place() {
    for ordering in WakeOrdering::ALL {
        let log = run(8, 5, 2, &[], ordering);
        let mut got = log.received.clone();
        got.sort_unstable();
        let want: Vec<(u32, u32)> = (0..8).flat_map(|w| (0..5).map(move |k| (w, k))).collect();
        assert_eq!(got, want, "{}: every message once", ordering.name());
        for w in 0..8 {
            let mine: Vec<u32> = log
                .received
                .iter()
                .filter(|m| m.0 == w)
                .map(|m| m.1)
                .collect();
            assert_eq!(
                mine,
                (0..5).collect::<Vec<_>>(),
                "{}: sender {w}'s order",
                ordering.name()
            );
        }
        // a woken sender always finds its place held: one wake per refusal, no refusal after one
        assert_eq!(
            log.capacity_wakes.len() as u32,
            log.refused,
            "{}: every refused sender was woken once, for its place",
            ordering.name()
        );
    }
}

#[test]
fn the_same_run_twice_is_the_same_run() {
    for ordering in WakeOrdering::ALL {
        let (a, b) = (run(6, 4, 1, &[], ordering), run(6, 4, 1, &[], ordering));
        assert_eq!(a.received, b.received);
        assert_eq!(a.capacity_wakes, b.capacity_wakes);
    }
}

#[test]
fn a_task_that_ends_holding_a_place_hands_it_on() {
    // senders 1 and 3 end as soon as a place is held for them, without sending. Their
    // CapacityWait is dropped with the task, so the place goes to the next waiter: every other
    // sender's messages still arrive and the run completes (a lost place would stall it).
    for ordering in WakeOrdering::ALL {
        let log = run(5, 3, 1, &[1, 3], ordering);
        assert_eq!(log.received.len(), 9, "{}", ordering.name());
        assert!(log.received.iter().all(|m| m.0 != 1 && m.0 != 3));
        let mut left = log.left.clone();
        left.sort_unstable();
        assert_eq!(
            left,
            [1, 3],
            "{}: both leavers were given a place",
            ordering.name()
        );
    }
}
