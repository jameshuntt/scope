//! The exact schedule, pinned. Every workload below runs under every `WakeOrdering` and three
//! poll budgets, twice: once with a tracer recording every hook, once with no tracer and the
//! tasks recording their own polls. The output must equal `tests/golden/schedule.txt`, which was
//! recorded from scope 0.3.0. Any change to who is polled when, how many rounds a run takes, or
//! what the tracer sees fails this test.
//!
//! Regenerate (only when a schedule change is intended): `UPDATE_GOLDEN=1 cargo test --test schedule_golden`.

use std::fmt::Write as _;
use std::time::Duration;

use scope::{
    CancelEscalation, CancelReason, Cx, QoSClass, Scope, ScopedTask, SchedulerConfig, TaskId, TaskPoll, TaskState,
    Tracer, WakeOrdering, WakeReason,
};

/// What the tasks share: their own record of who ran, and the ring's ids and hop budget.
#[derive(Default)]
struct Env {
    log: Vec<String>,
    ring: Vec<TaskId>,
    hops: u32,
}

/// Records every hook, in order.
struct FullTracer(std::rc::Rc<std::cell::RefCell<Vec<String>>>);

impl FullTracer {
    fn push(&self, line: String) {
        self.0.borrow_mut().push(line);
    }
}

impl Tracer for FullTracer {
    fn on_round_begin(&mut self, round: u64) {
        self.push(format!("round {round}"));
    }
    fn on_spawn(&mut self, id: TaskId, name: &str) {
        self.push(format!("spawn {id} {name}"));
    }
    fn on_wake(&mut self, id: TaskId, reason: WakeReason) {
        self.push(format!("wake {id} {reason:?}"));
    }
    fn on_poll_begin(&mut self, seq: u64, id: TaskId, name: &str) {
        self.push(format!("poll {seq} {id} {name}"));
    }
    fn on_poll_end(&mut self, id: TaskId, name: &str) {
        self.push(format!("end {id} {name}"));
    }
    fn on_finish(&mut self, id: TaskId, name: &str, state: TaskState) {
        self.push(format!("done {id} {name} {state:?}"));
    }
    fn on_subscribe_finish(&mut self, waiter: TaskId, target: TaskId) {
        self.push(format!("subscribe {waiter} {target}"));
    }
    fn on_defer_spawn(&mut self, parent: TaskId, child: &str) {
        self.push(format!("defer {parent} {child}"));
    }
    fn on_cancel_request(&mut self, id: TaskId, reason: CancelReason, ticks: Option<u64>, time: Option<Duration>) {
        self.push(format!("cancel_request {id} {reason:?} {ticks:?} {time:?}"));
    }
    fn on_cancel_hard_stop(&mut self, id: TaskId, reason: CancelReason) {
        self.push(format!("cancel {id} {reason:?}"));
    }
}

/// Yields `left` times, logging each poll.
struct Yielder {
    name: &'static str,
    left: u32,
}

impl<'env> ScopedTask<'env, Env> for Yielder {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, Env>) -> TaskPoll {
        cx.resources.log.push(format!("{} {}", self.name, self.left));
        if self.left == 0 {
            return TaskPoll::Ready;
        }
        self.left -= 1;
        cx.yield_now()
    }
}

/// One link of a wake ring: on each poll, spends a hop and wakes the next link; once the hops
/// run out, it finishes and wakes the next link so the whole ring drains.
struct RingLink {
    index: usize,
}

impl<'env> ScopedTask<'env, Env> for RingLink {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, Env>) -> TaskPoll {
        let env = &mut *cx.resources;
        env.log.push(format!("ring{} hops={}", self.index, env.hops));
        let next = env.ring[(self.index + 1) % env.ring.len()];
        if env.hops == 0 {
            cx.wake_with(next, WakeReason::Manual);
            return TaskPoll::Ready;
        }
        env.hops -= 1;
        cx.wake_with(next, WakeReason::Manual);
        TaskPoll::Pending
    }
}

/// Spawns `children` yielders on its first poll, then yields three more times and finishes.
struct Parent {
    children: u32,
    polls: u32,
}

impl<'env> ScopedTask<'env, Env> for Parent {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, Env>) -> TaskPoll {
        self.polls += 1;
        cx.resources.log.push(format!("parent {}", self.polls));
        if self.polls == 1 {
            for n in 0..self.children {
                let qos = [QoSClass::Batch, QoSClass::Interactive, QoSClass::Standard][n as usize % 3];
                cx.spawn_later_qos(format!("child{n}"), qos, Yielder { name: "child", left: n % 3 + 1 });
            }
        }
        if self.polls > 3 {
            return TaskPoll::Ready;
        }
        cx.yield_now()
    }
}

/// Loops yielding until it sees its own cancellation, then finishes.
struct Stubborn {
    polls: u32,
}

impl<'env> ScopedTask<'env, Env> for Stubborn {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, Env>) -> TaskPoll {
        self.polls += 1;
        cx.resources.log.push(format!("stubborn {} cancelled={}", self.polls, cx.is_cancelled()));
        if self.polls > 20 {
            return TaskPoll::Ready;
        }
        cx.yield_now()
    }
}

/// Cancels `target` on its second poll with a tick escalation.
struct Canceller {
    target: TaskId,
    polls: u32,
}

impl<'env> ScopedTask<'env, Env> for Canceller {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, Env>) -> TaskPoll {
        self.polls += 1;
        cx.resources.log.push(format!("canceller {}", self.polls));
        if self.polls == 2 {
            cx.cancel_escalate_with_reason(self.target, CancelEscalation::after_ticks(3), CancelReason::UserRequest);
            return TaskPoll::Ready;
        }
        cx.yield_now()
    }
}

const QOS: [QoSClass; 5] =
    [QoSClass::Background, QoSClass::Batch, QoSClass::Standard, QoSClass::Interactive, QoSClass::Realtime];

fn workload(name: &str, s: &mut Scope<'_, Env>) {
    match name {
        "yielders" => {
            const NAMES: [&str; 12] = ["y0", "y1", "y2", "y3", "y4", "y5", "y6", "y7", "y8", "y9", "y10", "y11"];
            for (i, n) in NAMES.iter().enumerate() {
                s.spawn_named_qos(*n, QOS[i % 5], Yielder { name: n, left: (i % 4) as u32 + 1 });
            }
        }
        "ring" => {
            for i in 0..6 {
                let id = s.spawn_named_qos(format!("ring{i}"), QOS[i % 5], RingLink { index: i });
                assert_eq!(id, TaskId::new(i as u32, 0));
            }
        }
        "spawn_join" => {
            s.spawn_named("parent", Parent { children: 7, polls: 0 });
            let a = s.spawn_with_result("a", Yielder { name: "a", left: 2 }, |t, _| t.left);
            let b = s.spawn_with_result("b", Yielder { name: "b", left: 4 }, |t, _| t.left);
            let c = s.spawn_with_result("c", Yielder { name: "c", left: 1 }, |t, _| t.left);
            let d = s.spawn_with_result("d", Yielder { name: "d", left: 3 }, |t, _| t.left);
            s.spawn_join_all("all", vec![a, b]);
            s.spawn_select2("sel", c, d);
        }
        "cancel" => {
            let target = s.spawn_named_qos("stubborn", QoSClass::Batch, Stubborn { polls: 0 });
            s.spawn_named("canceller", Canceller { target, polls: 0 });
            s.spawn_named("bystander", Yielder { name: "bystander", left: 6 });
        }
        other => panic!("unknown workload {other}"),
    }
}

fn run_once(name: &str, ordering: WakeOrdering, budget: usize, traced: bool) -> String {
    let mut env = Env::default();
    if name == "ring" {
        // A fresh scope hands out slots 0.. at generation 0, in spawn order.
        env.ring = (0..6).map(|slot| TaskId::new(slot, 0)).collect();
        env.hops = 15;
    }
    let lines = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let config = SchedulerConfig { wake_ordering: ordering, max_polls_per_round: budget, ..SchedulerConfig::default() };
    let report = {
        let mut s = Scope::with_config(&mut env, config);
        if traced {
            s.set_tracer(Box::new(FullTracer(lines.clone())));
        }
        workload(name, &mut s);
        s.run()
    };
    let mut out = String::new();
    writeln!(out, "== {name} {} budget={budget} traced={traced}", ordering.name()).unwrap();
    for line in lines.borrow().iter() {
        writeln!(out, "{line}").unwrap();
    }
    for line in &env.log {
        writeln!(out, "log {line}").unwrap();
    }
    writeln!(
        out,
        "report rounds={} polls={} completed={} cancelled={} panicked={} stalled={:?}",
        report.rounds, report.polls, report.completed, report.cancelled, report.panicked, report.stalled
    )
    .unwrap();
    out
}

fn all_schedules() -> String {
    let mut out = String::new();
    for name in ["yielders", "ring", "spawn_join", "cancel"] {
        for ordering in WakeOrdering::ALL {
            for budget in [usize::MAX, 3, 1] {
                for traced in [true, false] {
                    out.push_str(&run_once(name, ordering, budget, traced));
                }
            }
        }
    }
    out
}

#[test]
fn every_schedule_matches_the_recorded_one() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/schedule.txt");
    let now = all_schedules();
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden")).unwrap();
        std::fs::write(path, &now).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(path).expect("tests/golden/schedule.txt (UPDATE_GOLDEN=1 to record)");
    if now != golden {
        let first = now.lines().zip(golden.lines()).position(|(a, b)| a != b);
        panic!(
            "schedule changed; first differing line {:?}: now {:?}, recorded {:?}",
            first,
            first.and_then(|i| now.lines().nth(i)),
            first.and_then(|i| golden.lines().nth(i)),
        );
    }
}
