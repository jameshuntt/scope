//! Bounded mailboxes: a capacity fixed at spawn, `try_send` that enqueues or gives the message
//! back (`Full`, `Closed`), receiving frees capacity, and wakes exactly like the unbounded mailbox.

use std::cell::RefCell;
use std::rc::Rc;

use scope::{
    scoped, BoundedMailbox, BoundedSender, Cx, MailboxSender, SchedulerConfig, Scope, ScopedTask,
    TaskId, TaskPoll, TickOutcome, Tracer, TrySendError, WakeOrdering, WakeReason,
};

/// Takes at most `per_poll` messages a poll and finishes after `expect` of them.
struct Taker {
    rx: BoundedMailbox<u32>,
    per_poll: usize,
    seen: Vec<u32>,
    expect: usize,
}

impl<'env, R> ScopedTask<'env, R> for Taker {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        for _ in 0..self.per_poll {
            match self.rx.try_recv() {
                Some(m) => self.seen.push(m),
                None => break,
            }
        }
        if self.seen.len() >= self.expect {
            TaskPoll::Ready
        } else if self.rx.is_empty() {
            TaskPoll::Pending
        } else {
            // Messages are left and their wakes are spent: come back next round.
            cx.yield_now()
        }
    }
}

fn taker(rx: BoundedMailbox<u32>, expect: usize) -> Taker {
    Taker {
        rx,
        per_poll: usize::MAX,
        seen: Vec::new(),
        expect,
    }
}

#[test]
fn capacity_zero_refuses_every_send() {
    let mut env = ();
    scoped(&mut env, |s| {
        let (_id, tx) = s.spawn_with_bounded_mailbox("none", 0, |rx| taker(rx, 1));
        assert_eq!(tx.capacity(), 0);
        assert_eq!(tx.free(), 0);
        for m in 0..5 {
            assert_eq!(tx.try_send(m), Err(TrySendError::Full(m)));
        }
        assert_eq!(s.tick(), TickOutcome::Progress(1)); // the spawn poll: nothing arrived
        assert_eq!(s.tick(), TickOutcome::Idle); // no refusal woke it
        assert_eq!(tx.len(), 0);
    });
}

#[test]
fn exactly_capacity_is_accepted_and_one_more_is_full() {
    let mut env = ();
    let mut out = None;
    scoped(&mut env, |s| {
        let (handle, tx) = s.spawn_with_bounded_mailbox_result(
            "four",
            4,
            |rx| taker(rx, 4),
            |t, _| t.seen.clone(),
        );
        for m in 0..4 {
            assert_eq!(tx.try_send(m), Ok(()));
        }
        assert_eq!(tx.free(), 0);
        assert_eq!(tx.try_send(4), Err(TrySendError::Full(4)));
        assert_eq!(tx.try_send(5).map_err(TrySendError::into_inner), Err(5));
        assert_eq!(tx.len(), 4, "a refused message was not enqueued");
        let report = s.run();
        assert!(report.all_completed(), "{report:?}");
        out = handle.take();
    });
    assert_eq!(out, Some(Ok(vec![0, 1, 2, 3])));
}

#[test]
fn a_send_after_a_receive_is_accepted() {
    let mut env = ();
    scoped(&mut env, |s| {
        let (_id, tx) = s.spawn_with_bounded_mailbox("one", 1, |rx| Taker {
            rx,
            per_poll: 1,
            seen: Vec::new(),
            expect: 3,
        });
        assert_eq!(tx.try_send(10), Ok(()));
        assert_eq!(tx.try_send(11), Err(TrySendError::Full(11)));
        assert_eq!(s.tick(), TickOutcome::Progress(1)); // takes 10
        assert_eq!(tx.free(), 1, "receiving freed the place");
        assert_eq!(tx.try_send(11), Ok(()));
        assert_eq!(s.tick(), TickOutcome::Progress(1)); // takes 11
        assert_eq!(tx.try_send(12), Ok(()));
        assert_eq!(s.tick(), TickOutcome::Done);
    });
}

#[test]
fn a_closed_recipient_is_reported() {
    let mut env = ();
    scoped(&mut env, |s| {
        // Completed.
        let (_done, done_tx) = s.spawn_with_bounded_mailbox("done", 2, |rx| taker(rx, 1));
        assert_eq!(done_tx.try_send(1), Ok(()));
        // Hard-cancelled.
        let (cancelled, cancelled_tx) =
            s.spawn_with_bounded_mailbox("cancelled", 2, |rx| taker(rx, 9));
        // Closed by the receiver itself.
        struct Closer(BoundedMailbox<u32>);
        impl<'env, R> ScopedTask<'env, R> for Closer {
            fn poll(&mut self, _cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
                self.0.close();
                TaskPoll::Pending
            }
        }
        let (_closer, closer_tx) = s.spawn_with_bounded_mailbox("closer", 2, Closer);
        assert_eq!(s.tick(), TickOutcome::Progress(3));
        s.cancel_now(cancelled);
        assert_eq!(done_tx.try_send(2), Err(TrySendError::Closed(2)));
        assert_eq!(cancelled_tx.try_send(3), Err(TrySendError::Closed(3)));
        assert_eq!(closer_tx.try_send(4), Err(TrySendError::Closed(4)));
        assert!(done_tx.is_closed() && cancelled_tx.is_closed() && closer_tx.is_closed());
        assert_eq!(cancelled_tx.free(), 0);
    });
}

/// Records wakes and polls.
struct Lines(Rc<RefCell<Vec<String>>>);

impl Tracer for Lines {
    fn on_round_begin(&mut self, round: u64) {
        self.0.borrow_mut().push(format!("round {round}"));
    }
    fn on_wake(&mut self, id: TaskId, reason: WakeReason) {
        self.0.borrow_mut().push(format!("wake {id} {reason:?}"));
    }
    fn on_poll_begin(&mut self, seq: u64, id: TaskId, name: &str) {
        self.0.borrow_mut().push(format!("poll {seq} {id} {name}"));
    }
    fn on_finish(&mut self, id: TaskId, name: &str, state: scope::TaskState) {
        self.0
            .borrow_mut()
            .push(format!("done {id} {name} {state:?}"));
    }
}

/// What a producer sends through: either kind of sender.
enum Out {
    Unbounded(MailboxSender<u32>),
    Bounded(BoundedSender<u32>),
}

/// Sends `n` messages, one a poll; on `Full` it yields and tries the same message again.
struct Producer {
    out: Out,
    next: u32,
    n: u32,
    refused: Rc<RefCell<u32>>,
}

impl<'env, R> ScopedTask<'env, R> for Producer {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        if self.next == self.n {
            return TaskPoll::Ready;
        }
        match &self.out {
            Out::Unbounded(tx) => tx.send(self.next),
            Out::Bounded(tx) => match tx.try_send(self.next) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    *self.refused.borrow_mut() += 1;
                    return cx.yield_now();
                }
                Err(TrySendError::Closed(_)) => return TaskPoll::Ready,
            },
        }
        self.next += 1;
        cx.yield_now()
    }
}

/// Takes at most `per_poll` messages a poll from either kind of mailbox into a shared record.
enum In {
    Unbounded(scope::Mailbox<u32>),
    Bounded(BoundedMailbox<u32>),
}

struct Consumer {
    rx: In,
    per_poll: usize,
    seen: Rc<RefCell<Vec<u32>>>,
}

impl<'env, R> ScopedTask<'env, R> for Consumer {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        for _ in 0..self.per_poll {
            let m = match &self.rx {
                In::Unbounded(rx) => rx.try_recv(),
                In::Bounded(rx) => rx.try_recv(),
            };
            match m {
                Some(m) => self.seen.borrow_mut().push(m),
                None => break,
            }
        }
        let empty = match &self.rx {
            In::Unbounded(rx) => rx.is_empty(),
            In::Bounded(rx) => rx.is_empty(),
        };
        if self.seen.borrow().len() >= 30 {
            TaskPoll::Ready
        } else if empty {
            TaskPoll::Pending
        } else {
            // Messages are left and their wakes are spent: come back next round.
            cx.yield_now()
        }
    }
}

/// What one run gives: the trace, the messages in arrival order, the refusals, whether it ended
/// within the round limit.
type Run = (Vec<String>, Vec<u32>, u32, bool);

/// Three producers into one consumer, at most 400 rounds (`Lifo` with a budget of 1 starves the
/// consumer: that is the policy, and it repeats too).
fn run(ordering: WakeOrdering, budget: usize, capacity: Option<usize>, per_poll: usize) -> Run {
    let lines = Rc::new(RefCell::new(Vec::new()));
    let refused = Rc::new(RefCell::new(0));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut env = ();
    let config = SchedulerConfig::with_ordering(ordering).with_poll_budget(budget);
    let mut s = Scope::with_config(&mut env, config);
    s.set_tracer(Box::new(Lines(lines.clone())));
    let outs: Vec<Out> = match capacity {
        None => {
            let (_id, tx) = s.spawn_with_mailbox("consumer", |rx| Consumer {
                rx: In::Unbounded(rx),
                per_poll,
                seen: seen.clone(),
            });
            (0..3).map(|_| Out::Unbounded(tx.clone())).collect()
        }
        Some(cap) => {
            let (_id, tx) = s.spawn_with_bounded_mailbox("consumer", cap, |rx| Consumer {
                rx: In::Bounded(rx),
                per_poll,
                seen: seen.clone(),
            });
            (0..3).map(|_| Out::Bounded(tx.clone())).collect()
        }
    };
    for (i, out) in outs.into_iter().enumerate() {
        let first = i as u32 * 100;
        s.spawn_named(
            format!("p{i}"),
            Producer {
                out,
                next: first,
                n: first + 10,
                refused: refused.clone(),
            },
        );
    }
    let mut done = false;
    for _ in 0..400 {
        if s.tick() == TickOutcome::Done {
            done = true;
            break;
        }
    }
    drop(s);
    let lines = lines.borrow().clone();
    let seen = seen.borrow().clone();
    let refused = *refused.borrow();
    (lines, seen, refused, done)
}

#[test]
fn wake_ordering_is_the_unbounded_mailbox_s_when_nothing_is_refused() {
    for ordering in WakeOrdering::ALL {
        for budget in [usize::MAX, 3, 1] {
            let unbounded = run(ordering, budget, None, usize::MAX);
            let bounded = run(ordering, budget, Some(30), usize::MAX);
            assert_eq!(bounded.2, 0);
            assert_eq!(unbounded.3, bounded.3);
            assert_eq!(
                unbounded.0,
                bounded.0,
                "{} budget {budget}",
                ordering.name()
            );
            assert_eq!(unbounded.1, bounded.1);
        }
    }
}

#[test]
fn runs_with_refusals_repeat_exactly() {
    let mut refusals = 0;
    for ordering in WakeOrdering::ALL {
        for budget in [usize::MAX, 3, 1] {
            let first = run(ordering, budget, Some(2), 1);
            refusals += first.2;
            if first.3 {
                let mut sorted = first.1.clone();
                sorted.sort_unstable();
                let expected: Vec<u32> = (0..3).flat_map(|i| i * 100..i * 100 + 10).collect();
                assert_eq!(sorted, expected, "every message arrives once");
            }
            for _ in 0..9 {
                assert_eq!(
                    run(ordering, budget, Some(2), 1),
                    first,
                    "{} budget {budget}",
                    ordering.name()
                );
            }
        }
    }
    assert!(refusals > 0, "the workload hits the bound");
}
