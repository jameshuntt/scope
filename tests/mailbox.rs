//! Typed mailbox: deliver messages into a running scoped task.

use scope::{scoped, Cx, Mailbox, ScopedTask, TaskPoll};

// Sums messages until it has seen `expect` of them, then completes with the sum.
struct Summer {
    rx: Mailbox<i64>,
    total: i64,
    seen: usize,
    expect: usize,
}
impl<'env, R> ScopedTask<'env, R> for Summer {
    fn poll(&mut self, _cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        for m in self.rx.drain() {
            self.total += m;
            self.seen += 1;
        }
        if self.seen >= self.expect {
            TaskPoll::Ready
        } else {
            TaskPoll::Pending // parked until the next send wakes us
        }
    }
}

#[test]
fn messages_delivered_into_a_task_are_summed() {
    let mut env = ();
    let mut out = None;
    scoped(&mut env, |s| {
        let (handle, tx) = s.spawn_with_mailbox_result(
            "summer",
            |rx| Summer { rx, total: 0, seen: 0, expect: 3 },
            |t, _| t.total,
        );
        tx.send(10);
        tx.send(20);
        tx.send(12);
        let report = s.run();
        assert!(report.all_completed(), "{report:?}");
        out = handle.take();
    });
    assert_eq!(out, Some(Ok(42)));
}

// A producer task that posts N messages into a peer's mailbox, one per poll,
// then finishes.
struct Producer {
    tx: scope::MailboxSender<i64>,
    remaining: i64,
}
impl<'env, R> ScopedTask<'env, R> for Producer {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        if self.remaining == 0 {
            return TaskPoll::Ready;
        }
        self.tx.send(self.remaining);
        self.remaining -= 1;
        cx.yield_now()
    }
}

#[test]
fn one_task_can_post_into_another_tasks_mailbox() {
    let mut env = ();
    let mut out = None;
    scoped(&mut env, |s| {
        // Consumer expects 4 messages (values 4+3+2+1 = 10).
        let (handle, tx) = s.spawn_with_mailbox_result(
            "consumer",
            |rx| Summer { rx, total: 0, seen: 0, expect: 4 },
            |t, _| t.total,
        );
        s.spawn_named("producer", Producer { tx, remaining: 4 });
        let report = s.run();
        assert!(report.all_completed(), "{report:?}");
        out = handle.take();
    });
    assert_eq!(out, Some(Ok(10)));
}

#[test]
fn a_parked_mailbox_task_wakes_only_on_send() {
    use scope::TickOutcome;
    let mut env = ();
    scoped(&mut env, |s| {
        let (_id, tx) = s.spawn_with_mailbox(
            "sink",
            |rx| Summer { rx, total: 0, seen: 0, expect: 2 },
        );
        assert_eq!(s.tick(), TickOutcome::Progress(1)); // first poll: no msgs -> parks
        assert_eq!(s.tick(), TickOutcome::Idle); // still parked
        tx.send(5);
        assert_eq!(s.tick(), TickOutcome::Progress(1)); // woken by the send
        tx.send(7); // second message completes it
        assert_eq!(s.tick(), TickOutcome::Done);
    });
}
