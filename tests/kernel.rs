//! Integration tests for the scheduler core: cancellation, timers, join and
//! select, panic propagation, determinism, external wakers, stall reporting.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use scope::{
    run_scoped, scoped, Cx, JoinError, QoSClass, ScopedTask, SchedulerConfig, TaskPoll, TickOutcome,
    Tracer, WakeOrdering, YieldNowTask,
};

// A task that increments `R` (a counter) `n` times, one per poll, then finishes.
struct CountUp {
    remaining: u32,
}
impl<'env> ScopedTask<'env, u64> for CountUp {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, u64>) -> TaskPoll {
        if self.remaining == 0 {
            return TaskPoll::Ready;
        }
        *cx.resources += 1;
        self.remaining -= 1;
        cx.yield_now()
    }
}

#[test]
fn scoped_borrow_lets_many_tasks_mutate_env() {
    // Several tasks share &mut R with no locks and no 'static.
    let mut counter: u64 = 0;
    let report = run_scoped(&mut counter, |s| {
        s.spawn_named("a", CountUp { remaining: 10 });
        s.spawn_named("b", CountUp { remaining: 20 });
        s.spawn_named("c", CountUp { remaining: 5 });
    });
    assert_eq!(counter, 35);
    assert!(report.all_completed(), "{report:?}");
    assert_eq!(report.completed, 3);
}

// A task that returns a fixed value after `delay` polls.
struct Produce {
    delay: u32,
    value: i64,
}
impl<'env, R> ScopedTask<'env, R> for Produce {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        if self.delay == 0 {
            return TaskPoll::Ready;
        }
        self.delay -= 1;
        cx.yield_now()
    }
}

#[test]
fn join_all_collects_every_result() {
    let mut env = ();
    let mut results = None;
    scoped(&mut env, |s| {
        let a = s.spawn_with_result("a", Produce { delay: 1, value: 10 }, |t, _| t.value);
        let b = s.spawn_with_result("b", Produce { delay: 3, value: 20 }, |t, _| t.value);
        let c = s.spawn_with_result("c", Produce { delay: 0, value: 30 }, |t, _| t.value);
        let all = s.spawn_join_all("all", vec![a, b, c]);
        let rep = s.run();
        assert!(rep.all_completed(), "{rep:?}");
        results = all.take();
    });
    let got = results.expect("join_all produced").expect("no join error");
    let vals: Vec<i64> = got.into_iter().map(|r| r.unwrap()).collect();
    assert_eq!(vals, vec![10, 20, 30]);
}

#[test]
fn select2_reports_the_first_to_finish() {
    let mut env = ();
    let mut out = None;
    scoped(&mut env, |s| {
        let fast = s.spawn_with_result("fast", Produce { delay: 1, value: 1 }, |t, _| t.value);
        let slow = s.spawn_with_result("slow", Produce { delay: 9, value: 2 }, |t, _| t.value);
        let sel = s.spawn_select2("sel", fast, slow);
        s.run();
        out = sel.take();
    });
    match out.expect("select produced").expect("no join error") {
        scope::Either::Left(Ok(v)) => assert_eq!(v, 1),
        other => panic!("expected Left(Ok(1)), got {other:?}"),
    }
}

// Panics on its first poll.
struct Boom;
impl<'env, R> ScopedTask<'env, R> for Boom {
    fn poll(&mut self, _cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        panic!("boom");
    }
}

#[test]
fn a_panicking_task_becomes_a_join_error_and_isolates() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {})); // silence the deliberate panic
    let mut env = ();
    let mut boom_out = None;
    let mut ok_out = None;
    let report = scoped(&mut env, |s| {
        let boom = s.spawn_with_result("boom", Boom, |_t, _| 0i32);
        let ok = s.spawn_with_result("ok", Produce { delay: 2, value: 7 }, |t, _| t.value);
        let rep = s.run();
        boom_out = boom.take();
        ok_out = ok.take();
        rep
    });
    std::panic::set_hook(prev);

    assert_eq!(boom_out, Some(Err(JoinError::Panicked)));
    assert_eq!(ok_out, Some(Ok(7)));
    assert_eq!(report.panicked, 1);
    assert_eq!(report.completed, 1); // the sibling still finished
}

// Loops forever unless cancelled (relies on OutputTask's cooperative check).
struct LoopForever;
impl<'env, R> ScopedTask<'env, R> for LoopForever {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        cx.yield_now()
    }
}

#[test]
fn cooperative_cancel_yields_cancelled_join_error() {
    use scope::CancelReason;
    let mut env = ();
    let mut out = None;
    let report = scoped(&mut env, |s| {
        let h = s.spawn_with_result("loop", LoopForever, |_t, _| 0i32);
        let id = h.task;
        // Let it run a couple of rounds, then ask it to stop.
        for _ in 0..2 {
            let _ = s.tick();
        }
        s.request_cancel(id, CancelReason::UserRequest);
        let rep = s.run();
        out = h.take();
        rep
    });
    assert_eq!(out, Some(Err(JoinError::Cancelled(CancelReason::UserRequest))));
    assert_eq!(report.cancelled, 1);
}

// Ignores cancellation entirely (raw spawn, no cooperative wrapper).
struct Stubborn;
impl<'env, R> ScopedTask<'env, R> for Stubborn {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        cx.yield_now()
    }
}

#[test]
fn tick_budget_hard_stops_a_stubborn_task() {
    use scope::CancelReason;
    let mut env = ();
    let report = scoped(&mut env, |s| {
        s.set_default_cancel_after_ticks(3);
        let id = s.spawn_named("stubborn", Stubborn);
        s.request_cancel(id, CancelReason::UserRequest);
        s.run() // terminates because the tick deadline fires
    });
    assert_eq!(report.cancelled, 1);
    assert_eq!(report.completed, 0);
}

#[test]
fn sleep_task_completes_after_its_delay() {
    let mut env = ();
    let report = scoped(&mut env, |s| {
        s.sleep("nap", Duration::from_millis(15));
        s.run()
    });
    assert!(report.all_completed(), "{report:?}");
    assert_eq!(report.completed, 1);
}

// Parks (never self-wakes); completes on its second poll only.
struct ParkOnce {
    polled: u32,
}
impl<'env, R> ScopedTask<'env, R> for ParkOnce {
    fn poll(&mut self, _cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        self.polled += 1;
        if self.polled >= 2 {
            TaskPoll::Ready
        } else {
            TaskPoll::Pending // parked: needs an external wake
        }
    }
}

#[test]
fn a_real_external_waker_resumes_a_parked_task() {
    use scope::WakeReason;
    let mut env = ();
    scoped(&mut env, |s| {
        let h = s.spawn_with_result("park", ParkOnce { polled: 0 }, |t, _| t.polled);
        let waker = s.waker_for(h.task, WakeReason::Io);

        assert_eq!(s.tick(), TickOutcome::Progress(1)); // 1st poll -> parks
        assert_eq!(s.tick(), TickOutcome::Idle); // parked: no wake => no progress

        waker.wake(); // fire from "outside", like an IO reactor would

        // Resumed, polled, and completed in this round — slab empties => Done.
        // Without the wake, the previous line proves it would have stayed Idle.
        assert_eq!(s.tick(), TickOutcome::Done);
        assert_eq!(h.take(), Some(Ok(2)));
    });
}

// Parks on its first poll and hands its real waker to a thread that fires it
// shortly after. Once woken it hard-stops the long keepalive sleep.
struct WokenFromAThread {
    keepalive: scope::TaskId,
    started: bool,
}
impl<'env, R> ScopedTask<'env, R> for WokenFromAThread {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        if !self.started {
            self.started = true;
            let waker = cx.waker();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(50));
                waker.wake();
            });
            return TaskPoll::Pending;
        }
        cx.cancel_escalate_with_reason(
            self.keepalive,
            scope::CancelEscalation::immediate(),
            scope::CancelReason::UserRequest,
        );
        TaskPoll::Ready
    }
}

#[test]
fn a_wake_from_another_thread_during_runs_idle_wait_is_not_lost() {
    // `run` blocks on the bus until the 10 s sleep is due; the other thread's
    // wake lands during that wait and must still reach the task.
    let start = std::time::Instant::now();
    let mut env = ();
    let report = run_scoped(&mut env, |s| {
        let keepalive = s.sleep("keepalive", Duration::from_secs(10)).task;
        s.spawn_named("woken", WokenFromAThread { keepalive, started: false });
    });
    assert!(report.stalled.is_empty(), "{report:?}");
    assert_eq!((report.completed, report.cancelled), (1, 1), "{report:?}");
    assert!(start.elapsed() < Duration::from_secs(5), "the run waited out the sleep: {report:?}");
}

// Parks forever and never arranges a wake — a buggy task.
struct DeadPark;
impl<'env, R> ScopedTask<'env, R> for DeadPark {
    fn poll(&mut self, _cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        TaskPoll::Pending
    }
}

#[test]
fn a_stalled_task_is_reported_not_hung() {
    let mut env = ();
    let report = scoped(&mut env, |s| {
        s.spawn_named("dead", DeadPark);
        s.spawn_named("live", YieldNowTask::new());
        s.run() // must return, not hang
    });
    assert_eq!(report.completed, 1); // the live one finished
    assert_eq!(report.stalled.len(), 1);
    assert_eq!(report.stalled[0].1, "dead");
}

// Records a deterministic signature of scheduling order.
#[derive(Clone)]
struct SigTracer(Arc<Mutex<Vec<String>>>);
impl Tracer for SigTracer {
    fn on_poll_begin(&mut self, _seq: u64, id: scope::TaskId, name: &str) {
        self.0.lock().unwrap().push(format!("poll {id} {name}"));
    }
    fn on_finish(&mut self, id: scope::TaskId, name: &str, state: scope::TaskState) {
        self.0.lock().unwrap().push(format!("done {id} {name} {state:?}"));
    }
}

fn run_signature(ordering: WakeOrdering) -> Vec<String> {
    let sink = Arc::new(Mutex::new(Vec::new()));
    let mut env: u64 = 0;
    let mut scope = scope::Scope::with_config(&mut env, SchedulerConfig::with_ordering(ordering));
    scope.set_tracer(Box::new(SigTracer(sink.clone())));
    scope.spawn_named_qos("hi", QoSClass::Interactive, CountUp { remaining: 3 });
    scope.spawn_named_qos("lo", QoSClass::Background, CountUp { remaining: 3 });
    scope.spawn_named_qos("mid", QoSClass::Standard, CountUp { remaining: 3 });
    let _ = scope.run();
    let out = sink.lock().unwrap().clone();
    out
}

#[test]
fn scheduling_is_deterministic_per_policy() {
    for ordering in WakeOrdering::ALL {
        let a = run_signature(ordering);
        let b = run_signature(ordering);
        assert_eq!(a, b, "ordering {:?} was not deterministic", ordering.name());
        assert!(!a.is_empty());
    }
}

#[test]
fn qos_bias_polls_high_priority_first() {
    // Under BiasedQoS the Interactive task should be polled before Background
    // on the very first round.
    let sig = run_signature(WakeOrdering::BiasedQoS);
    let first_hi = sig.iter().position(|l| l.contains("hi")).unwrap();
    let first_lo = sig.iter().position(|l| l.contains("lo")).unwrap();
    assert!(first_hi < first_lo, "high QoS should lead: {sig:?}");
}

// Holds its OWN `&'env mut Vec` in the task struct — a mutable borrow of stack
// data kept across suspension points (each `yield_now` is a suspension), while
// other tasks interleave. This is the property async Rust cannot give a
// spawned task, demonstrated directly.
struct PushInto<'a> {
    buf: &'a mut Vec<u32>,
    tag: u32,
    remaining: u32,
}
impl<'env> ScopedTask<'env, Vec<u32>> for PushInto<'env> {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, Vec<u32>>) -> TaskPoll {
        if self.remaining == 0 {
            return TaskPoll::Ready;
        }
        self.buf.push(self.tag); // through the borrow held across yields
        cx.resources.push(self.tag); // and into the shared env, to prove interleaving
        self.remaining -= 1;
        cx.yield_now()
    }
}

#[test]
fn tasks_hold_distinct_mut_borrows_of_stack_data_across_suspension() {
    let mut a: Vec<u32> = Vec::new();
    let mut b: Vec<u32> = Vec::new();
    let mut order: Vec<u32> = Vec::new();

    let report = run_scoped(&mut order, |s| {
        s.spawn_named("a", PushInto { buf: &mut a, tag: 1, remaining: 3 });
        s.spawn_named("b", PushInto { buf: &mut b, tag: 2, remaining: 3 });
    });
    assert!(report.all_completed(), "{report:?}");

    // Each task wrote through its own &mut borrow...
    assert_eq!(a, vec![1, 1, 1]);
    assert_eq!(b, vec![2, 2, 2]);
    // ...and the shared-env trace proves the borrows were held *across*
    // suspensions while the two tasks interleaved, not run back-to-back.
    assert_eq!(order, vec![1, 2, 1, 2, 1, 2]);
}

// A parent that spawns a child mid-poll, then finishes.
struct Parent;
impl<'env> ScopedTask<'env, u64> for Parent {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, u64>) -> TaskPoll {
        cx.spawn_later_named("child", CountUp { remaining: 4 });
        TaskPoll::Ready
    }
}

#[test]
fn a_task_can_spawn_a_child_into_the_scope() {
    let mut counter: u64 = 0;
    let report = run_scoped(&mut counter, |s| {
        s.spawn_named("parent", Parent);
    });
    assert_eq!(counter, 4);
    assert_eq!(report.completed, 2); // parent + child
}
