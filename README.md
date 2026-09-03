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
  **timers** (`SleepTask`), **join / select / join-all** (`JoinHandle`,
  `Either`), **panic isolation** (a panicking task fails its join, the scope
  survives), and **typed mailboxes** (`Mailbox`, `MailboxSender`) for delivering
  messages into a running task from any thread.
- **Tracing** (`Tracer`, `BufferTracer`, `LogTracer`, `CountingTracer`) for
  determinism checks: two runs of the same program produce the same trace.

## Determinism and verification

The runtime is single-threaded and its scheduling is a pure function of the
configured policy and the order of wakes, so a program run twice produces the
same interleaving. The test suite covers cancellation, timers, join and select,
panic propagation, external wakers, mailboxes and stall reporting; the
cross-thread wake and cancel handoff is model-checked with Loom
(`RUSTFLAGS="--cfg loom" cargo test --test loom_wake --release`), and the kernel
runs clean under Miri.

## What it is not

A thread pool. Tasks borrow the environment because exactly one thread polls
them; a work-stealing variant would be a different runtime, not a feature of
this one. Use OS threads around scopes (one runtime per thread, as the 1BRC
example does), not scopes across threads.

## License

MIT OR Apache-2.0.
