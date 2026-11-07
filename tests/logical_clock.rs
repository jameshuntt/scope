//! The logical clock: a tick is a scheduler round, logical timers fire at the start of their round
//! as `Timer` wakes on the bus, and a run with them repeats tick for tick under every policy.

use std::cell::RefCell;
use std::rc::Rc;

use scope::{
    CancelReason, Cx, Mailbox, QoSClass, SchedulerConfig, Scope, ScopedTask, TaskId, TaskPoll,
    Tick, TickDelta, TickOutcome, Tracer, WakeOrdering, WakeReason,
};

/// Records the hooks a logical-timer run touches, in order.
struct Lines(Rc<RefCell<Vec<String>>>);

impl Tracer for Lines {
    fn on_round_begin(&mut self, round: u64) {
        self.0.borrow_mut().push(format!("round {round}"));
    }
    fn on_spawn(&mut self, id: TaskId, name: &str) {
        self.0.borrow_mut().push(format!("spawn {id} {name}"));
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
    fn on_sleep_ticks(&mut self, id: TaskId, at: Tick) {
        self.0.borrow_mut().push(format!("sleep_ticks {id} {at}"));
    }
    fn on_tick_timer(&mut self, id: TaskId, at: Tick) {
        self.0.borrow_mut().push(format!("tick_timer {id} {at}"));
    }
    fn on_cancel_hard_stop(&mut self, id: TaskId, reason: CancelReason) {
        self.0.borrow_mut().push(format!("cancel {id} {reason:?}"));
    }
}

/// What the tasks write: `(task name, tick of the poll)` for every poll.
#[derive(Default)]
struct Env {
    polls: Vec<(String, u64)>,
}

/// Sleeps `delta` ticks `times` times, logging the tick of each poll.
struct Napper {
    name: &'static str,
    delta: u64,
    times: u32,
}

impl<'env> ScopedTask<'env, Env> for Napper {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, Env>) -> TaskPoll {
        let now = cx.now_tick().0;
        cx.resources.polls.push((self.name.to_owned(), now));
        if self.times == 0 {
            return TaskPoll::Ready;
        }
        self.times -= 1;
        cx.sleep_ticks(TickDelta(self.delta));
        TaskPoll::Pending
    }
}

/// Yields `left` times, logging each poll's tick.
struct Yielder {
    name: &'static str,
    left: u32,
}

impl<'env> ScopedTask<'env, Env> for Yielder {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, Env>) -> TaskPoll {
        let now = cx.now_tick().0;
        cx.resources.polls.push((self.name.to_owned(), now));
        if self.left == 0 {
            return TaskPoll::Ready;
        }
        self.left -= 1;
        cx.yield_now()
    }
}

#[test]
fn a_tick_is_the_number_of_the_round() {
    let mut env = Env::default();
    let mut s = Scope::new(&mut env);
    assert_eq!(s.now_tick(), Tick(0));
    s.spawn_named("y", Yielder { name: "y", left: 3 });
    for round in 1..=4 {
        assert!(matches!(
            s.tick(),
            TickOutcome::Progress(1) | TickOutcome::Done
        ));
        assert_eq!(s.now_tick(), Tick(round));
    }
    drop(s);
    let ticks: Vec<u64> = env.polls.iter().map(|(_, t)| *t).collect();
    assert_eq!(ticks, [1, 2, 3, 4], "each poll sees the tick of its round");
}

#[test]
fn a_logical_timer_wakes_the_task_in_the_round_with_its_tick() {
    let lines = Rc::new(RefCell::new(Vec::new()));
    let mut env = Env::default();
    let report = {
        let mut s = Scope::new(&mut env);
        s.set_tracer(Box::new(Lines(lines.clone())));
        s.spawn_named(
            "napper",
            Napper {
                name: "napper",
                delta: 5,
                times: 2,
            },
        );
        s.run()
    };
    assert!(report.all_completed(), "{report:?}");
    // Polled in round 1, asleep 5 ticks: round 6, then round 11. Idle rounds in between are rounds.
    assert_eq!(
        env.polls,
        [
            ("napper".to_owned(), 1),
            ("napper".to_owned(), 6),
            ("napper".to_owned(), 11)
        ]
    );
    assert_eq!(report.rounds, 11);
    let lines = lines.borrow();
    let at6: Vec<&str> = lines
        .iter()
        .skip_while(|l| *l != "round 6")
        .take(4)
        .map(String::as_str)
        .collect();
    // The timer fires first, then the wake comes off the bus like every other wake, then the poll.
    assert_eq!(
        at6,
        [
            "round 6",
            "tick_timer #0.0 t6",
            "wake #0.0 Timer",
            "poll 2 #0.0 napper"
        ]
    );
    assert!(lines.contains(&"sleep_ticks #0.0 t6".to_owned()));
    assert!(lines.contains(&"sleep_ticks #0.0 t11".to_owned()));
}

#[test]
fn zero_ticks_is_the_next_round_and_a_past_tick_is_the_next_round() {
    struct Asks(u32);
    impl<'env> ScopedTask<'env, Env> for Asks {
        fn poll(&mut self, cx: &mut Cx<'_, 'env, Env>) -> TaskPoll {
            let now = cx.now_tick().0;
            cx.resources.polls.push(("asks".to_owned(), now));
            self.0 += 1;
            match self.0 {
                1 => cx.sleep_ticks(TickDelta(0)),
                2 => cx.sleep_until_tick(Tick(1)),
                3 => cx.sleep_until_tick(Tick(now + 4)),
                _ => return TaskPoll::Ready,
            }
            TaskPoll::Pending
        }
    }
    let mut env = Env::default();
    scope::run_scoped(&mut env, |s| {
        s.spawn_named("asks", Asks(0));
    });
    let ticks: Vec<u64> = env.polls.iter().map(|(_, t)| *t).collect();
    assert_eq!(ticks, [1, 2, 3, 7]);
}

#[test]
fn a_later_request_replaces_the_earlier_one() {
    struct Twice(bool);
    impl<'env> ScopedTask<'env, Env> for Twice {
        fn poll(&mut self, cx: &mut Cx<'_, 'env, Env>) -> TaskPoll {
            let now = cx.now_tick().0;
            cx.resources.polls.push(("twice".to_owned(), now));
            if self.0 {
                return TaskPoll::Ready;
            }
            self.0 = true;
            cx.sleep_ticks(TickDelta(9));
            cx.sleep_ticks(TickDelta(3));
            TaskPoll::Pending
        }
    }
    let lines = Rc::new(RefCell::new(Vec::new()));
    let mut env = Env::default();
    let report = {
        let mut s = Scope::new(&mut env);
        s.set_tracer(Box::new(Lines(lines.clone())));
        s.spawn_named("twice", Twice(false));
        s.run()
    };
    assert_eq!(
        env.polls.iter().map(|(_, t)| *t).collect::<Vec<_>>(),
        [1, 4]
    );
    // The tick-10 timer is stale once the task is gone: run ends at round 4, not 10.
    assert_eq!(report.rounds, 4);
    assert_eq!(
        lines
            .borrow()
            .iter()
            .filter(|l| l.starts_with("tick_timer"))
            .count(),
        1
    );
}

#[test]
fn a_cancelled_sleeper_does_not_keep_run_going() {
    struct Parked {
        rx: Mailbox<()>,
    }
    impl<'env> ScopedTask<'env, Env> for Parked {
        fn poll(&mut self, _cx: &mut Cx<'_, 'env, Env>) -> TaskPoll {
            let _ = self.rx.try_recv();
            TaskPoll::Pending
        }
    }
    let mut env = Env::default();
    let mut s = Scope::new(&mut env);
    let sleeper = s.spawn_named(
        "sleeper",
        Napper {
            name: "sleeper",
            delta: 1_000_000,
            times: 1,
        },
    );
    let (_parked, _tx) = s.spawn_with_mailbox("parked", |rx| Parked { rx });
    assert_eq!(s.tick(), TickOutcome::Progress(2));
    s.cancel_now(sleeper);
    let report = s.run();
    // One parked task is reported, after one more round, not a million.
    assert_eq!(report.stalled.len(), 1);
    assert_eq!(report.rounds, 2);
}

#[test]
fn tick_sleep_tasks_join_like_sleep_tasks() {
    let mut env = Env::default();
    let mut s = Scope::new(&mut env);
    let a = s.sleep_ticks("a", TickDelta(4));
    let b = s.sleep_until_tick("b", Tick(9));
    let zero = s.sleep_ticks("zero", TickDelta(0));
    let report = s.run();
    assert!(report.all_completed(), "{report:?}");
    assert_eq!(a.take(), Some(Ok(())));
    assert_eq!(b.take(), Some(Ok(())));
    assert_eq!(zero.take(), Some(Ok(())));
    assert_eq!(report.rounds, 9, "b completes in round 9");
}

/// Napping and yielding tasks of mixed QoS whose timers land in rounds busy with other wakes.
fn mixed(s: &mut Scope<'_, Env>) {
    const QOS: [QoSClass; 5] = [
        QoSClass::Background,
        QoSClass::Batch,
        QoSClass::Standard,
        QoSClass::Interactive,
        QoSClass::Realtime,
    ];
    const NAP: [&str; 6] = ["n0", "n1", "n2", "n3", "n4", "n5"];
    const YIELD: [&str; 4] = ["y0", "y1", "y2", "y3"];
    for (i, name) in NAP.iter().enumerate() {
        s.spawn_named_qos(
            *name,
            QOS[i % 5],
            Napper {
                name,
                delta: 1 + (i as u64 % 3),
                times: 3,
            },
        );
    }
    for (i, name) in YIELD.iter().enumerate() {
        s.spawn_named_qos(
            *name,
            QOS[(i + 2) % 5],
            Yielder {
                name,
                left: 4 + i as u32,
            },
        );
    }
}

fn run_mixed(ordering: WakeOrdering, budget: usize) -> (Vec<String>, Vec<(String, u64)>, u64) {
    let lines = Rc::new(RefCell::new(Vec::new()));
    let mut env = Env::default();
    let config = SchedulerConfig::with_ordering(ordering).with_poll_budget(budget);
    let report = {
        let mut s = Scope::with_config(&mut env, config);
        s.set_tracer(Box::new(Lines(lines.clone())));
        mixed(&mut s);
        s.run()
    };
    assert!(report.all_completed(), "{report:?}");
    let lines = lines.borrow().clone();
    (lines, env.polls, report.rounds)
}

#[test]
fn same_inputs_and_policy_give_the_same_ticks() {
    let mut per_policy = Vec::new();
    for ordering in WakeOrdering::ALL {
        for budget in [usize::MAX, 3, 1] {
            let first = run_mixed(ordering, budget);
            for _ in 0..9 {
                assert_eq!(
                    run_mixed(ordering, budget),
                    first,
                    "{} budget {budget}",
                    ordering.name()
                );
            }
            per_policy.push(first);
        }
    }
    // The policy decides the interleaving: not every configuration gives the same trace.
    assert!(per_policy.iter().any(|run| run.0 != per_policy[0].0));
}

#[test]
fn timer_wakes_are_ordered_by_the_policy_like_any_wake() {
    // Under a poll budget of 1, two timers due in one round and a yielder compete: the policy,
    // not the timer, picks who goes first.
    let order = |ordering| {
        let (_, polls, _) = {
            let lines = Rc::new(RefCell::new(Vec::new()));
            let mut env = Env::default();
            let config = SchedulerConfig::with_ordering(ordering).with_poll_budget(1);
            let report = {
                let mut s = Scope::with_config(&mut env, config);
                s.set_tracer(Box::new(Lines(lines.clone())));
                s.spawn_named_qos(
                    "low",
                    QoSClass::Background,
                    Napper {
                        name: "low",
                        delta: 2,
                        times: 1,
                    },
                );
                s.spawn_named_qos(
                    "high",
                    QoSClass::Realtime,
                    Napper {
                        name: "high",
                        delta: 2,
                        times: 1,
                    },
                );
                s.spawn_named_qos("y", QoSClass::Standard, Yielder { name: "y", left: 6 });
                s.run()
            };
            assert!(report.all_completed());
            let l = lines.borrow().clone();
            (l, env.polls, report.rounds)
        };
        polls
            .into_iter()
            .map(|(n, t)| format!("{n}@{t}"))
            .collect::<Vec<_>>()
    };
    let fifo = order(WakeOrdering::Fifo);
    let qos = order(WakeOrdering::BiasedQoS);
    assert_ne!(fifo, qos);
    // Under BiasedQoS the Realtime napper is always polled before the Background one.
    let first =
        |v: &[String], who: &str| v.iter().skip(2).position(|p| p.starts_with(who)).unwrap();
    assert!(first(&qos, "high") < first(&qos, "low"), "{qos:?}");
}
