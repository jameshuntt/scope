# scope

A single-threaded, cooperative, **deterministic** structured-concurrency
runtime whose tasks may borrow non-`'static` data from the scope environment,
the thing spawned `async` tasks cannot do.

Every task is polled `&mut self` and handed a context carrying `&mut R`, a live
exclusive borrow of whatever the scope owns. No `Arc<Mutex<_>>`, no `'static`,
no locks: polling is cooperative and single-threaded, so the borrow is sound by
construction. The scope boundary is a join point; `run` drives every task to
completion and reports stalled tasks instead of hanging.

`unsafe` is confined to one audited module (`wake`, the `waker_tables` plumbing
that mints real `std::task::Waker`s); everything else is `#![deny(unsafe_code)]`.

## When to use it

Reach for scope when all three of these are true: the work is naturally many
small cooperative steps, those steps share one piece of mutable state, and you
want the same program to schedule the same way every time.

- **Simulations and engines.** A world, a game state, a document model: many
  actors mutate one structure. Here each actor is a task and the structure is
  `R`; no actor holds a lock, no actor holds an `Arc`, and the frame order is
  the scheduling policy you chose, not the operating system's.
- **Protocol and session state machines.** Each connection is a task borrowing
  the shared tables it needs. External IO wakes the task through a real
  `Waker`; the parsing and the table updates stay single-threaded and
  lock-free.
- **Tests and replays.** Because scheduling is deterministic, a failing
  interleaving is a fixed input, not a flaky one. Two runs with the same
  policy produce the same trace, and `BufferTracer` lets a test assert on it.
- **Embedded-style control loops** that want cancellation with escalation,
  timers, join and select, without a thread pool underneath.

How it differs from the things you already know:

- An async executor (tokio, smol) spawns `'static` futures onto threads; the
  price of that freedom is `Arc<Mutex<State>>` around anything shared and no
  guaranteed interleaving. scope gives up the threads and keeps the borrow.
- `std::thread::scope` and rayon's `scope` let borrowed work run in parallel,
  which means shared state still needs synchronization. scope runs the work
  cooperatively on one thread, so `&mut R` is enough.
- A hand-rolled `loop { for task in &mut tasks { task.step(&mut state) } }`
  is what scope is, plus the parts you would write next: wake-driven
  readiness instead of polling everyone, real wakers for the outside world,
  cancellation, timers, join and select, stall detection, and policies that
  keep any one task from starving the rest.

If you need work to run in parallel across cores, use threads and put one
scope on each, as the 1BRC example does.

## Quick start

```rust
use scope::{run_scoped, Cx, ScopedTask, TaskPoll};

// A task that adds to the shared counter once per poll, `n` times.
struct CountUp {
    remaining: u32,
}

impl<'env> ScopedTask<'env, u64> for CountUp {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, u64>) -> TaskPoll {
        if self.remaining == 0 {
            return TaskPoll::Ready;
        }
        *cx.resources += 1; // `&mut u64`, borrowed from the scope, no lock
        self.remaining -= 1;
        cx.yield_now()
    }
}

let mut counter: u64 = 0;
let report = run_scoped(&mut counter, |s| {
    s.spawn_named("a", CountUp { remaining: 10 });
    s.spawn_named("b", CountUp { remaining: 20 });
    s.spawn_named("c", CountUp { remaining: 5 });
});
assert_eq!(counter, 35);
assert!(report.all_completed());
```

Tasks can also return values and be joined, selected or joined in bulk:

```rust
use scope::{scoped, Cx, ScopedTask, TaskPoll};

struct Produce { delay: u32, value: i64 }

impl<'env, R> ScopedTask<'env, R> for Produce {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        if self.delay == 0 { return TaskPoll::Ready; }
        self.delay -= 1;
        cx.yield_now()
    }
}

let mut env = ();
let mut results = None;
scoped(&mut env, |s| {
    let a = s.spawn_with_result("a", Produce { delay: 1, value: 10 }, |t, _| t.value);
    let b = s.spawn_with_result("b", Produce { delay: 3, value: 20 }, |t, _| t.value);
    let all = s.spawn_join_all("all", vec![a, b]);
    let report = s.run();
    assert!(report.all_completed(), "{report:?}");
    results = all.take();
});
let values: Vec<i64> = results
    .expect("join_all produced")
    .expect("no join error")
    .into_iter()
    .map(|r| r.expect("each task produced"))
    .collect();
assert_eq!(values, vec![10, 20]);
```

## What is in the box

- **Scoped tasks** (`ScopedTask`, `Cx`): polled with `&mut R`; `yield_now`,
  sleep, cancellation checks and task-local wake reasons come through the context.
- **A real waker substrate** (`WakeBus`, `scheduler_waker`): genuine
  `std::task::Waker`s addressed by generational `TaskId`, so IO, timers and other
  threads can wake a scoped task. Stale wakes after a slot is reused resolve to
  nothing.
- **Policy-driven scheduling** (`SchedulerConfig`, `WakeOrdering`, `QoSClass`):
  FIFO, LIFO, round-robin, QoS-biased and QoS-then-age orderings, plus a
  poll budget per round. The default was chosen by running the candidates
  against deterministic workload suites.
- **Cancellation** with tick and wall-clock escalation (`CancelEscalation`),
  **timers** on the wall clock (`SleepTask`) and on the logical clock (`TickSleepTask`,
  `Cx::sleep_ticks`), **join / select / join-all** (`JoinHandle`,
  `Either`), **panic isolation** (a panicking task fails its join, the scope
  survives), and **typed mailboxes** (`Mailbox`, `MailboxSender`) for delivering
  messages into a running task from any thread, unbounded or bounded (`BoundedMailbox`,
  `BoundedSender::try_send`).
- **Tracing** (`Tracer`, `BufferTracer`, `LogTracer`, `CountingTracer`) for
  determinism checks: two runs of the same program produce the same trace.

## The logical clock and bounded mailboxes

A `Tick` is the number of a scheduler round. The clock advances by one at the start of every round
(each `tick()`, each loop of `run`), and nothing else moves it. `Cx::sleep_ticks(TickDelta(n))` wakes
the task at the start of the round `n` after this one, as a `Timer` wake on the bus, ordered by the
policy like every other wake. Nothing reads the wall clock, so a run that uses only logical timers
repeats tick for tick. `SleepTask` and `Cx::sleep_until` stay as they are, on the wall clock. The
clock never wraps: it stops at `u64::MAX`, and a deadline past it is `u64::MAX`.

`Scope::spawn_with_bounded_mailbox(name, capacity, build)` gives a mailbox that holds at most
`capacity` messages. `BoundedSender::try_send` enqueues and wakes like `send`, or hands the message
back as `TrySendError::Full` or `TrySendError::Closed` (the task is gone) and changes nothing.

```rust
use scope::{scoped, BoundedMailbox, Cx, ScopedTask, TaskPoll, TickDelta, TrySendError};

// Takes one message, then sleeps three rounds on the logical clock, then finishes.
struct Slow {
    rx: BoundedMailbox<u32>,
    got: Option<u32>,
}

impl<'env> ScopedTask<'env, Vec<u64>> for Slow {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, Vec<u64>>) -> TaskPoll {
        cx.resources.push(cx.now_tick().0);
        if self.got.is_some() {
            return TaskPoll::Ready;
        }
        self.got = self.rx.try_recv();
        cx.sleep_ticks(TickDelta(3));
        TaskPoll::Pending
    }
}

let mut ticks = Vec::new();
scoped(&mut ticks, |s| {
    let (_id, tx) = s.spawn_with_bounded_mailbox("slow", 1, |rx| Slow { rx, got: None });
    assert_eq!(tx.try_send(7), Ok(()));
    assert_eq!(tx.try_send(8), Err(TrySendError::Full(8)));
    assert!(s.run().all_completed());
    assert_eq!(tx.try_send(9), Err(TrySendError::Closed(9)));
});
assert_eq!(ticks, vec![1, 4]);
```

### Waiting for a place

A sender refused `Full` can wait for a place instead of trying again blind.
`BoundedSender::wake_when_free(task)` registers the task in the mailbox, after every task already
waiting there. When a receive frees a place (or a drain frees several), the mailbox holds each freed
place for the oldest waiter, one place each, and only then wakes it with `WakeReason::Capacity`. One
freed place wakes one waiter, never all of them. The woken task fills its place with
`try_send_from(msg, task)`; a sender that does not wait still uses every place nobody holds, but it
cannot take a held one. `BoundedSender::free_for(task)` counts the places a task may fill, its held
place included, for a caller that checks before it sends.

`wake_when_free` checks under the mailbox's lock: if a place is free for the task it registers
nothing and answers `FreeNow` (send again), so a place freed between a refused send and the
registration is never missed. A task waits once per mailbox (`AlreadyRegistered` keeps its turn),
and a closed mailbox answers `Closed`. Closing (or the receiver ending) wakes every waiter and lets
every held place go.

`Registered` carries a `CapacityWait`. Keep it in the waiting task: dropping it gives the
registration up, and a place held for the task goes to the next waiter. Scope drops a task's state
when the task ends by any path (it completes, is cancelled or panics), so a task that ends after it
was woken but before it sent hands its place on instead of losing it. A registration of a slot's
newer generation drops the older generation's entry and held place, so a stale `TaskId` never takes
a place from the slot's next task.

```rust
use scope::{
    scoped, BoundedMailbox, BoundedSender, CapacityWait, Cx, ScopedTask, TaskPoll, TrySendError,
    WaitRegistration,
};

// Sends 0, 1, 2 into a mailbox of one place; when it is full, waits for a place.
struct Sender {
    tx: BoundedSender<u32>,
    next: u32,
    wait: Option<CapacityWait<u32>>,
}

impl<'env, R> ScopedTask<'env, R> for Sender {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        while self.next < 3 {
            match self.tx.try_send_from(self.next, cx.id()) {
                Ok(()) => {
                    self.next += 1;
                    self.wait = None;
                }
                Err(TrySendError::Full(_)) => match self.tx.wake_when_free(cx.id()) {
                    WaitRegistration::Registered(w) => {
                        self.wait = Some(w);
                        return TaskPoll::Pending;
                    }
                    WaitRegistration::AlreadyRegistered => return TaskPoll::Pending,
                    WaitRegistration::FreeNow => {}
                    WaitRegistration::Closed => return TaskPoll::Ready,
                },
                Err(TrySendError::Closed(_)) => return TaskPoll::Ready,
            }
        }
        TaskPoll::Ready
    }
}

// Takes one message a poll into the environment.
struct Taker {
    rx: BoundedMailbox<u32>,
}

impl<'env> ScopedTask<'env, Vec<u32>> for Taker {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, Vec<u32>>) -> TaskPoll {
        if let Some(m) = self.rx.try_recv() {
            cx.resources.push(m);
        }
        if cx.resources.len() == 3 {
            TaskPoll::Ready
        } else if self.rx.is_empty() {
            TaskPoll::Pending
        } else {
            cx.yield_now()
        }
    }
}

let mut got = Vec::new();
scoped(&mut got, |s| {
    let (_id, tx) = s.spawn_with_bounded_mailbox("taker", 1, |rx| Taker { rx });
    s.spawn_named("sender", Sender { tx, next: 0, wait: None });
    assert!(s.run().all_completed());
});
assert_eq!(got, [0, 1, 2]);
```

## Determinism and verification

The runtime is single-threaded and its scheduling is a pure function of the
configured policy and the order of wakes, so a program run twice produces the
same interleaving. With logical timers, the tick at which each thing happens is
a function of the wake sequence, the policy and the tick-advancement rule (one
tick per round), and repeats with them. Wall-clock timers (`SleepTask`,
`Cx::sleep_until`) and cancel escalation by time are outside that replay: how
many rounds pass while they wait depends on elapsed time. The test suite covers cancellation, timers, join and select,
panic propagation, external wakers, mailboxes and stall reporting; the
cross-thread wake and cancel handoff is model-checked with Loom
(`RUSTFLAGS="--cfg loom" cargo test --test loom_wake --release`), and the kernel
runs clean under Miri.

GitHub Actions run all of this on every push (the tests including the pinned schedule, clippy,
docs, Rust 1.85, Miri, loom, and the NEON example on an ARM runner), and publish a `v*` tag only when
every check passes.

## What it is not

A thread pool. Tasks borrow the environment because exactly one thread polls
them; a work-stealing variant would be a different runtime, not a feature of
this one. Use OS threads around scopes (one runtime per thread, as the 1BRC
example does), not scopes across threads.

## 0.4.0

- Capacity waiters on the bounded mailbox: `BoundedSender::wake_when_free`, `try_send_from`,
  `free_for`, `WaitRegistration`, `CapacityWait` and `WakeReason::Capacity`. A freed place is held for
  the oldest waiting sender and wakes only it; a held place goes to the next waiter when its
  `CapacityWait` is dropped, and close wakes every waiter. Without waiters the bounded mailbox does
  what it did in 0.3.2.
- Breaking only because `WakeReason` gains a variant: a `match` on it with no `_` arm needs one more.
- Not published yet (no tag); see CHANGELOG.md.

## 0.3.2

- A logical clock: `Tick` is the number of a scheduler round, one per round, tick 0 before the first.
  `Cx::sleep_ticks`, `Cx::sleep_until_tick`, `TickSleepTask` and `Scope::sleep_ticks` wake a task at
  the start of a round as a `Timer` wake on the bus, ordered by the policy like any wake; ties in the
  same round go in `TaskId` order. Nothing on that path reads the wall clock. The clock stops at
  `u64::MAX` rather than wrap.
- A bounded mailbox: `Scope::spawn_with_bounded_mailbox` and `BoundedSender::try_send`, which hands
  the message back as `Full` or `Closed` and enqueues, wakes and allocates nothing then.
- Additive only. `Mailbox`, `SleepTask` and the schedule are unchanged: `tests/schedule_golden.rs`
  passes as recorded from 0.3.0. The two new `Tracer` hooks have default bodies.

## 0.3.1

- The scheduler takes 12% to 42% less time on the crate's benchmarks (`cargo bench --bench
  scheduler`, against 0.3.0): 33% to 42% for many tasks yielding under the default policy, 25% to 41%
  for wake-driven work among many idle tasks, 22% to 27% for spawning, 12% to 16% for join and
  mailboxes. A round now costs the runnable tasks instead of every live one; a poll no longer copies
  the task's name or touches reference counts; the wake queue is drained in one lock.
- Fixed: when `run()` or `run_until()` had nothing to poll and waited, a wake from another thread that
  ended the wait was taken off the queue and dropped, so the task was reported stalled.
- The schedule is unchanged: `tests/schedule_golden.rs` pins every poll, wake, spawn, finish and round
  for 4 workloads under all 5 policies and 3 poll budgets, recorded from 0.3.0.
- The borrowing guarantee now has a real test: the `run_scoped` doc example that must fail to compile
  fails only for "does not live long enough" (E0597); before, it failed for an unrelated type error and
  would have passed even for a legal borrow.

## 0.3.0

Built on `waker_tables` 0.3: `scheduler_waker` gets a vtable built from `WakeSignal` itself, so
the waker can never run another type's functions (0.1.4 took the vtable as a separate argument).
Because `WakeSignal` implements `waker_tables`' public trait, this is a minor version. Minimum
Rust is 1.85, which `waker_tables` (edition 2024) already required.

## License

MIT OR Apache-2.0.
