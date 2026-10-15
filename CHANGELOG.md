# Changelog

Every code link below is pinned to a commit, so it opens the lines as they were at that commit. A
"Before" link shows the code with the problem and an "After" link shows the change. Publish times come
from the crates.io API, in UTC. Programs and scripts that are not part of the crate are in
[`proofs/`](proofs/README). Each one has a README with the command and what it prints.

## Unreleased

### Added

- **CI.** `.github/workflows/ci.yml` runs on pushes to `main`, on `v*` tags, on pull requests and once a
  week. Its jobs:
  - `test`: the tests (the pinned schedule included), the doc tests, clippy and rustdoc with
    `-D warnings`, `cargo bench --no-run` and `cargo package`.
  - `arm`: the tests and clippy on an ARM runner. The 1BRC example's NEON code only builds on aarch64.
  - `msrv`: the tests on Rust 1.85.
  - `miri`: the lib tests and the kernel tests under Miri, with `sleep_task` skipped.
  - `loom`: `tests/loom_wake.rs` and the lib tests built with `--cfg loom`, and clippy on that lib build.
  - `latest-deps`: `cargo update`, then the tests, on stable and on 1.85. It runs on pushes and the
    weekly run, and a publish does not wait for it.
  - `publish`: only on a `v*` tag. It needs test, arm, msrv, miri and loom to pass, and the tag to match
    the version in `Cargo.toml`.
  - [.github/workflows/ci.yml L1-117 @ 782814c](https://github.com/jameshuntt/scope/blob/782814cc9bd7b5cb8a0746d46113e7e6da7c8f34/.github/workflows/ci.yml#L1-L117): the whole workflow.
  - README note: [README.md L148-150 @ 782814c](https://github.com/jameshuntt/scope/blob/782814cc9bd7b5cb8a0746d46113e7e6da7c8f34/README.md#L148-L150), the README's paragraph on CI. It says "on every push";
    pushes to branches other than `main` do not run it.
  - Commit [782814c](https://github.com/jameshuntt/scope/commit/782814cc9bd7b5cb8a0746d46113e7e6da7c8f34).
  - Proof: run [38692079734](https://github.com/jameshuntt/scope/actions/runs/38692079734) on 782814c
    passed every job (test, arm, msrv, miri, loom, latest-deps on stable and on 1.85). The publish job was
    skipped because the push was not a tag.

## 0.3.1 (2026-10-15 06:17 UTC)

Published from [e9f73c3](https://github.com/jameshuntt/scope/commit/e9f73c31020cd59736b8b59c47a930f153cf7063). The package's `.cargo_vcs_info.json` names that commit. Its commits, oldest
first: 19b484b, cfb873a, 513cb50, da655cf, c4fe22d, 967b616, 407684b, e9f73c3.

**The borrow is unchanged.** `ScopedTask<'env, R>` and `Cx`'s `pub resources: &'poll mut R` read the same
as in 0.3.0.

- [src/task.rs L64-74 @ 6fce0fe](https://github.com/jameshuntt/scope/blob/6fce0feab3bac3e107f9cff0763cf92dc5922659/src/task.rs#L64-L74): `ScopedTask` and the start of `Cx` in 0.3.0.
- [src/task.rs L63-73 @ e9f73c3](https://github.com/jameshuntt/scope/blob/e9f73c31020cd59736b8b59c47a930f153cf7063/src/task.rs#L63-L73): the same lines in 0.3.1.

The only change inside `Cx` is crate-private. `bus` and `cancel` went from owned values (`WakeBus`,
`Arc<CancelCell>`) to borrows for the poll (`&'poll WakeBus`, `&'poll CancelCell`).

- Before: [src/task.rs L77-78 @ 6fce0fe](https://github.com/jameshuntt/scope/blob/6fce0feab3bac3e107f9cff0763cf92dc5922659/src/task.rs#L77-L78), the owned fields.
- After: [src/task.rs L76-79 @ e9f73c3](https://github.com/jameshuntt/scope/blob/e9f73c31020cd59736b8b59c47a930f153cf7063/src/task.rs#L76-L79), the borrowed fields.

We checked it three ways:

- No line declaring a public item changed. This prints two lines, the `WakeBus::recv_timeout` signature
  removed in one place and added in another with the same text:

  ```
  git diff 6fce0fe e9f73c3 -- src | grep '^[-+]' | grep pub | grep -v 'pub(crate)'
  ```
- The borrow tests pass on 0.3.1.
  - [tests/kernel.rs L27-39 @ e9f73c3](https://github.com/jameshuntt/scope/blob/e9f73c31020cd59736b8b59c47a930f153cf7063/tests/kernel.rs#L27-L39): `scoped_borrow_lets_many_tasks_mutate_env`. Three tasks share
    `&mut u64` with no lock.
  - [tests/kernel.rs L326-365 @ e9f73c3](https://github.com/jameshuntt/scope/blob/e9f73c31020cd59736b8b59c47a930f153cf7063/tests/kernel.rs#L326-L365): `tasks_hold_distinct_mut_borrows_of_stack_data_across_suspension`.
    Each task holds its own `&'env mut Vec` across yields while they interleave.
- The `run_scoped` doc examples now pin the bound from both sides. A borrow of data that outlives the scope
  compiles. The same task borrowing data that dies first fails with E0597 and nothing else. See Fixed
  below.

No line containing `unsafe` in `src/` changed from 0.2.2 to 0.3.1.
`git diff d2b31b7 782814c -- src | grep '^[-+].*unsafe'` prints two lines, the old and new wording of one
doc comment in `src/scope.rs`:

```
-//! * Zero `unsafe`: the mid-poll callbacks record *intents* into local buffers
+//! * Zero `unsafe`: the mid-poll callbacks record *intents* into buffers
```

Everything outside `wake` is still `#![deny(unsafe_code)]`.

### Added

- **The exact schedule is pinned.** `tests/schedule_golden.rs` runs 4 workloads: yielders, a wake ring,
  spawn and join, and cancel with tick escalation. Each one runs under all 5 `WakeOrdering`s and poll
  budgets `usize::MAX`, 3 and 1. Each of those runs twice, once with a tracer recording every hook and
  once with the tasks logging their own polls. The output has to equal `tests/golden/schedule.txt`: 120
  runs, 10,631 lines, recorded from 0.3.0. 19b484b's `src/` is identical to 6fce0fe's. If any of these
  runs polls a different task, takes a different number of rounds or shows the tracer something
  different, the test fails. A change that none of the 4 workloads reaches would still pass.
  - [tests/schedule_golden.rs L1-7 @ 19b484b](https://github.com/jameshuntt/scope/blob/19b484b84cff1f4853ce89602ddb5b5822bc8b67/tests/schedule_golden.rs#L1-L7): what the test covers, and how to record it again.
  - [tests/schedule_golden.rs L231-264 @ 19b484b](https://github.com/jameshuntt/scope/blob/19b484b84cff1f4853ce89602ddb5b5822bc8b67/tests/schedule_golden.rs#L231-L264): the loops over workloads, orderings and budgets, and the
    comparison.
  - Commit [19b484b](https://github.com/jameshuntt/scope/commit/19b484b84cff1f4853ce89602ddb5b5822bc8b67).
  - Proof: `cargo test --test schedule_golden` passes at 19b484b, cfb873a, 513cb50, da655cf, c4fe22d,
    967b616, 407684b and e9f73c3, each built in its own fresh target dir. `tests/golden/schedule.txt` is
    the same file at all of them.
  - Proof that it catches a change: [`proofs/golden-lifo`](proofs/golden-lifo/README) gives
    `WakeOrdering::Lifo` the FIFO key in `order_key` and runs the test.
    [src/scope.rs L136 @ e9f73c3](https://github.com/jameshuntt/scope/blob/e9f73c31020cd59736b8b59c47a930f153cf7063/src/scope.rs#L136) is the `Lifo` key it changes. The test fails with
    `schedule changed; first differing line Some(753): now Some("poll 1 #0.0 y0"), recorded Some("poll 1 #11.0 y11")`.
    `Some(753)` counts from 0, so that is line 754 of `tests/golden/schedule.txt`.
- **Scheduler benchmarks.** `benches/scheduler.rs` uses criterion. `yield_storm` has 10, 100 and 1000
  tasks yielding 20 times, under `BiasedQoSThenAge`, `RoundRobin` and `Fifo`. `sparse_wake_ring` is a
  ring of 10, 100 or 1000 links passing 2000 wakes. The others are `spawn_churn`, `join_all` and
  `mailbox`. Each iteration builds the scope, spawns and runs to completion, so spawn and finish are in
  the number. Run them with `cargo bench --bench scheduler`. They are in the repo, not in the published
  package: `include` in `Cargo.toml` has no `benches/**`.
  - [benches/scheduler.rs L1-200 @ 19b484b](https://github.com/jameshuntt/scope/blob/19b484b84cff1f4853ce89602ddb5b5822bc8b67/benches/scheduler.rs#L1-L200): the whole bench file.
  - [Cargo.toml L27-31 @ 19b484b](https://github.com/jameshuntt/scope/blob/19b484b84cff1f4853ce89602ddb5b5822bc8b67/Cargo.toml#L27-L31): the criterion dev-dependency and the `[[bench]]` entry.
  - Commit [19b484b](https://github.com/jameshuntt/scope/commit/19b484b84cff1f4853ce89602ddb5b5822bc8b67).
  - Problem, not fixed yet: `sparse_wake_ring` is not sparse. Its comment says only one link is runnable
    at a time. But every link starts runnable and each poll wakes the next link, so every round polls
    every link. [`proofs/ring-count`](proofs/ring-count/README) runs the same workload on 0.3.1 and
    prints `links=10 rounds=201 polls=2010`, `links=100 rounds=21 polls=2100` and
    `links=1000 rounds=3 polls=3000`. The README's "wake-driven work among many idle tasks" is this
    bench, so that label is wrong too. Its numbers are for a ring where every task is runnable.
  - [benches/scheduler.rs L25-26 @ 19b484b](https://github.com/jameshuntt/scope/blob/19b484b84cff1f4853ce89602ddb5b5822bc8b67/benches/scheduler.rs#L25-L26): the comment that says one link is runnable at a time.
  - [README.md L158-159 @ e9f73c3](https://github.com/jameshuntt/scope/blob/e9f73c31020cd59736b8b59c47a930f153cf7063/README.md#L158-L159): the README label.
  - Not in 0.2.2. It came in with 19b484b, part of 0.3.1. The case the comment describes is measured by
    the sparse probe, under the runnable set entry below.

### Changed

The scheduler is faster with the same schedule. [README.md L155-168 @ e9f73c3](https://github.com/jameshuntt/scope/blob/e9f73c31020cd59736b8b59c47a930f153cf7063/README.md#L155-L168) is the README's 0.3.1
section, with its numbers against 0.3.0. From 0.2.2 to 0.3.0 the only file in `src/` that changed is
`src/wake.rs` (the waker vtable), so 0.3.0's scheduler code is 0.2.2's.

We ran `cargo bench --bench scheduler` again for this changelog. It ran at 19b484b, cfb873a, 513cb50,
da655cf, c4fe22d, 967b616 and e9f73c3, in that order, and then at 19b484b a second time. 407684b was
not benched; outside test builds its code is the same as 967b616's. It was one Apple Silicon Mac
(aarch64-apple-darwin, rustc 1.97.1, criterion defaults) with other work running. 0.3.0 is 19b484b
(the same `src/` as 6fce0fe, plus the bench) and 0.3.1 is e9f73c3. The logs, the loop that made them
and a script that compares two of them are in
[`proofs/bench-per-commit`](proofs/bench-per-commit/README).

Time per iteration is criterion's point estimate. The last column is how far the same bench moved
between the two runs of 19b484b, 23 minutes apart, with no code change.

| bench | 0.3.0 | 0.3.1 | change | 19b484b run twice |
|---|---:|---:|---:|---:|
| `yield_storm/biased_qos_then_age/10` | 12.4 µs | 8.1 µs | -34.6% | -1.5% |
| `yield_storm/biased_qos_then_age/100` | 121.0 µs | 73.2 µs | -39.5% | +4.1% |
| `yield_storm/biased_qos_then_age/1000` | 1.339 ms | 775.9 µs | -42.1% | +3.1% |
| `yield_storm/round_robin/10` | 12.2 µs | 8.4 µs | -31.0% | 0.0% |
| `yield_storm/round_robin/100` | 107.3 µs | 73.9 µs | -31.1% | -0.5% |
| `yield_storm/round_robin/1000` | 1.009 ms | 784.8 µs | -22.2% | +0.1% |
| `yield_storm/fifo/10` | 11.6 µs | 8.0 µs | -31.0% | -0.8% |
| `yield_storm/fifo/100` | 96.6 µs | 72.0 µs | -25.5% | +0.9% |
| `yield_storm/fifo/1000` | 1.017 ms | 780.3 µs | -23.2% | -3.7% |
| `sparse_wake_ring/10` | 103.6 µs | 62.2 µs | -40.0% | -0.3% |
| `sparse_wake_ring/100` | 113.4 µs | 72.5 µs | -36.0% | +8.2% |
| `sparse_wake_ring/1000` | 263.6 µs | 197.4 µs | -25.1% | +11.4% |
| `spawn_churn/100` | 15.0 µs | 11.6 µs | -22.8% | +8.2% |
| `spawn_churn/1000` | 140.1 µs | 99.5 µs | -28.9% | +2.7% |
| `spawn_churn/10000` | 1.382 ms | 1.073 ms | -22.4% | +7.2% |
| `join_all/10` | 5.0 µs | 4.2 µs | -15.4% | +11.1% |
| `join_all/100` | 39.0 µs | 34.5 µs | -11.5% | +18.1% |
| `join_all/1000` | 383.0 µs | 328.4 µs | -14.2% | -0.2% |
| `mailbox/100` | 3.0 µs | 2.5 µs | -14.8% | -1.4% |
| `mailbox/10000` | 207.1 µs | 172.9 µs | -16.5% | -2.1% |

How far the numbers move with no code change: the two runs of 19b484b moved by -3.7% to +18.1% (last
column). 967b616 and e9f73c3, whose `src/` differs only in doc comments and a test-only `cfg`, moved by
-2.0% to +4.6%. A change no bigger than what its own bench moved with no code change proves nothing by
itself. In the table that is `join_all/100`: -11.5%, where two runs of the same code moved +18.1%.

That is close to what the README says. `yield_storm` under the default `BiasedQoSThenAge` takes 34.6%
to 42.1% less time (README: 33% to 42%). `sparse_wake_ring` 25.1% to 40.0% (README: 25% to 41%).
`spawn_churn` 22.4% to 28.9% (README: 22% to 27%). `join_all` and `mailbox` 11.5% to 16.5% (README: 12%
to 16%).

The gains we claim come from three commits: cfb873a, 513cb50 and 967b616. Each entry below gives its
lines and what it changed on its own, measured against the commit before it. c4fe22d moved every bench
by 4.3% or less, and we do not claim a speed change for it.

- **Poll path: no name clone, no `Arc` traffic, reused intent buffers.** Every poll cloned the task's
  name (`e.name.clone()`, a `String` allocation) and its cancel cell's `Arc`. It built `Cx` with another
  clone of the bus `Arc`. Both `Arc`s were dropped again when the poll ended. It also made four new
  `Vec`s for the intents, so a poll that spawned, slept, subscribed or cancelled allocated them fresh.
  - Before: [src/scope.rs L695-701 @ 19b484b](https://github.com/jameshuntt/scope/blob/19b484b84cff1f4853ce89602ddb5b5822bc8b67/src/scope.rs#L695-L701), the name and the cancel cell cloned at the start of the
    poll.
  - Before: [src/scope.rs L716-721 @ 19b484b](https://github.com/jameshuntt/scope/blob/19b484b84cff1f4853ce89602ddb5b5822bc8b67/src/scope.rs#L716-L721), the four new intent `Vec`s.
  - Before: [src/scope.rs L749-759 @ 19b484b](https://github.com/jameshuntt/scope/blob/19b484b84cff1f4853ce89602ddb5b5822bc8b67/src/scope.rs#L749-L759), `Cx` built with `bus.clone()` and `cell.clone()`.
  - Before: [src/task.rs L77-78 @ 19b484b](https://github.com/jameshuntt/scope/blob/19b484b84cff1f4853ce89602ddb5b5822bc8b67/src/task.rs#L77-L78), `Cx`'s owned `bus` and `cancel` fields.

  Now `Cx` borrows the bus and the cell for the poll. The tracer borrows the name from the slab entry,
  and only when a tracer is attached. The four buffers live in the `Scope` as `PollIntents` and keep
  their capacity. `insert_task` moves the name into the entry instead of cloning it. A child spawned
  mid-poll gets its cancel cell when the spawn is applied, not inside the hook.
  - After: [src/task.rs L76-79 @ cfb873a](https://github.com/jameshuntt/scope/blob/cfb873ab14af169304d80b81bac2f75211ec8bb0/src/task.rs#L76-L79), `Cx`'s fields as `&'poll WakeBus` and `&'poll CancelCell`.
  - After: [src/scope.rs L94-109 @ cfb873a](https://github.com/jameshuntt/scope/blob/cfb873ab14af169304d80b81bac2f75211ec8bb0/src/scope.rs#L94-L109), `PollIntents`, the four buffers kept in the `Scope`.
  - After: [src/scope.rs L717-748 @ cfb873a](https://github.com/jameshuntt/scope/blob/cfb873ab14af169304d80b81bac2f75211ec8bb0/src/scope.rs#L717-L748), one `get_mut` to start the poll, the tracer borrowing the
    name, the buffers lent out with `std::mem::take`, and the shared borrow of the slab for the poll.
  - After: [src/scope.rs L773-790 @ cfb873a](https://github.com/jameshuntt/scope/blob/cfb873ab14af169304d80b81bac2f75211ec8bb0/src/scope.rs#L773-L790), `Cx` built from `bus` and `&entry.cancel`, with no clones.
  - After: [src/scope.rs L259-278 @ cfb873a](https://github.com/jameshuntt/scope/blob/cfb873ab14af169304d80b81bac2f75211ec8bb0/src/scope.rs#L259-L278), `insert_task` moving the name into the entry, and the tracer
    reading it from there.
  - After: [src/scope.rs L811 @ cfb873a](https://github.com/jameshuntt/scope/blob/cfb873ab14af169304d80b81bac2f75211ec8bb0/src/scope.rs#L811), the spawned child's cancel cell made when the spawn is applied.
  - Commit [cfb873a](https://github.com/jameshuntt/scope/commit/cfb873ab14af169304d80b81bac2f75211ec8bb0).

  The entry the cell is borrowed from stays in the slab for the whole poll, because the slab is borrowed
  shared for that long. The compiler checks it, and `#![deny(unsafe_code)]` covers `scope.rs`.
  - Proof: schedule golden passes at cfb873a. Measured against 19b484b: `yield_storm` 13.5% to 21.4% less
    time, `sparse_wake_ring` 19.3% to 22.2%, `spawn_churn` 19.3% to 23.5%, `join_all` 12.6% to 14.7%.
    `join_all/100`'s 13.9% is less than the 18.1% that bench moved between two runs of 19b484b, so it
    proves nothing by itself. `join_all/10`'s 12.6% is only a little more than its 11.1%. `mailbox`
    moved 2.0% and 2.9%, near the 1.4% and 2.1% it moved between the two runs of 19b484b, and we do not
    claim it.
  - c4fe22d's commit message also lists the `insert_task` name move. That line changed here, in cfb873a.
- **Wake bus: one lock per drain, no notify when nobody waits.** `drain_bus` took wakes off the bus one
  `try_recv` at a time, so N queued wakes cost N + 1 locks of the bus mutex. `send` called `notify_one`
  on every send, with or without a receiver blocked on the condvar.
  - Before: [src/scope.rs L640-658 @ cfb873a](https://github.com/jameshuntt/scope/blob/cfb873ab14af169304d80b81bac2f75211ec8bb0/src/scope.rs#L640-L658), `drain_bus` taking one wake per `try_recv`.
  - Before: [src/wake.rs L97-115 @ cfb873a](https://github.com/jameshuntt/scope/blob/cfb873ab14af169304d80b81bac2f75211ec8bb0/src/wake.rs#L97-L115), `send` calling `notify_one` every time, and `try_recv`
    locking once per message.

  Now `drain_bus` swaps its empty `VecDeque` (`wake_batch`) for the bus queue under one lock. It applies
  the batch with the lock released, oldest first, as before. With a tracer attached it takes the queue
  once more after applying. A tracer hook is the only code on the scheduler thread that can send while a
  batch is applied, so this keeps those wakes in the same round, as taking them one at a time did.
  `send` calls `notify_one` only when `Queue::waiting` is above zero. A receiver counts itself under the
  lock before its wait releases the lock, so a sender that reads 0 has nobody to wake.
  - After: [src/scope.rs L644-684 @ 513cb50](https://github.com/jameshuntt/scope/blob/513cb506a7cbec53351d27459db014224f2abef8/src/scope.rs#L644-L684), `drain_bus` taking the whole queue with `take_all`, and
    `apply_wake_batch`.
  - After: [src/wake.rs L69-77 @ 513cb50](https://github.com/jameshuntt/scope/blob/513cb506a7cbec53351d27459db014224f2abef8/src/wake.rs#L69-L77), `Queue` and its `waiting` count.
  - After: [src/wake.rs L110-118 @ 513cb50](https://github.com/jameshuntt/scope/blob/513cb506a7cbec53351d27459db014224f2abef8/src/wake.rs#L110-L118), `send` notifying only when `waiting > 0`.
  - After: [src/wake.rs L138-168 @ 513cb50](https://github.com/jameshuntt/scope/blob/513cb506a7cbec53351d27459db014224f2abef8/src/wake.rs#L138-L168), `take_all` swapping the buffers, and `wait_nonempty` counting
    the receiver before it waits.
  - Commit [513cb50](https://github.com/jameshuntt/scope/commit/513cb506a7cbec53351d27459db014224f2abef8).
  - Behaviour change: with no tracer, a wake that another thread sends after the swap now waits for the
    next round. Before, `drain_bus` kept taking wakes until the queue was empty, so such a wake could
    still land in the same round. We have not reproduced this; it depends on thread timing. Wakes sent
    from the scheduler's own thread are not affected, and the recorded schedule is the same.
  - Proof: two unit tests, [src/wake.rs L245-290 @ 513cb50](https://github.com/jameshuntt/scope/blob/513cb506a7cbec53351d27459db014224f2abef8/src/wake.rs#L245-L290), `take_all_keeps_fifo_order_behind_leftovers`
    and `a_blocked_receiver_is_woken_by_a_send_from_another_thread`.
  - Proof: two loom models, [src/wake.rs L293-346 @ 513cb50](https://github.com/jameshuntt/scope/blob/513cb506a7cbec53351d27459db014224f2abef8/src/wake.rs#L293-L346), a blocked receiver against one sender and
    against two. loom's `wait_timeout` never times out, so a skipped notify shows up as a deadlock.
    `verify.sh` runs them ([verify.sh L21 @ 513cb50](https://github.com/jameshuntt/scope/blob/513cb506a7cbec53351d27459db014224f2abef8/verify.sh#L21), the loom lib line), and so does the CI loom job.
    `RUSTFLAGS="--cfg loom" cargo test --lib --release` at e9f73c3 gives 4 passed: both models and the
    two `ids` tests.
  - The models catch a lost wake. [`proofs/loom-notify`](proofs/loom-notify/README) changes
    `if q.waiting > 0` to `if q.waiting > 1` in `send` and both models fail. [src/wake.rs L115 @ e9f73c3](https://github.com/jameshuntt/scope/blob/e9f73c31020cd59736b8b59c47a930f153cf7063/src/wake.rs#L115)
    is the line it changes. The one-sender model reports
    `deadlock; threads = [(Id(0), Blocked(Location(None))), (Id(1), Terminated)]`.
  - Schedule golden passes at 513cb50. Measured against cfb873a: `mailbox` 14.0% and 16.1% less time.
    That bench sends 100 or 10,000 messages before `run()`, and each send queues a wake on the bus. Every
    other bench moved between -6.6% and +3.8%, and we do not claim those.
- **Round overhead: clock only for a pending deadline, one slab lookup per id, smaller task entry.**
  Every round read the clock up to three times with no timer or cancel deadline pending.
  `fire_expired_cancel_times`, `fire_expired_timers` and `next_deadline` each called `Instant::now()`
  before looking at their heap. The poll loop and the timer pass looked a task up twice, with `get` to
  check it and `get_mut` to change it. The wake apply called `get_mut` twice. `TaskEntry` was 144 bytes
  and a slab slot 152, with `cancel_tick_deadline` an `Option<u64>`. `build_report` went through
  `live_ids()` and looked every id up again. `live_ids` walked every slot to the end of the slab.
  - Before: [src/scope.rs L952-953 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L952-L953), `fire_expired_cancel_times` reading the clock first.
  - Before: [src/scope.rs L974-975 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L974-L975), `fire_expired_timers` reading the clock first.
  - Before: [src/scope.rs L995-996 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L995-L996), `next_deadline` reading the clock first.
  - Before: [src/scope.rs L623-633 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L623-L633), the poll loop: `get`, then `get_mut`.
  - Before: [src/scope.rs L667-678 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L667-L678), the wake apply: `get_mut` twice.
  - Before: [src/scope.rs L981-990 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L981-L990), the timer pass: `get`, then `get_mut`.
  - Before: [src/scope.rs L73-86 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L73-L86), `TaskEntry` with `cancel_tick_deadline: Option<u64>`.
  - Before: [src/ids.rs L32-35 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/ids.rs#L32-L35), `Slot`.
  - Before: [src/ids.rs L120-128 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/ids.rs#L120-L128), `live_ids` walking every slot.
  - Before: [src/scope.rs L1008-1014 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L1008-L1014), `build_report` looking each id up again.

  Now the timer and cancel passes return before reading the clock when their heap is empty.
  `next_deadline` returns `None` before reading it. The poll loop, the wake apply and the timer pass look
  each id up once, and so do `cancel_now_with_reason`, `request_cancel` and `schedule_cancel_deadlines`.
  `TaskEntry` is `repr(C)`, with what a round reads for every live task first: `runnable_seq`,
  `last_polled`, `name`, `runnable`, `state`, `qos`. `cancel_tick_deadline` is `Option<NonZeroU64>`. The
  deadline is the tick count plus a budget of at least one, so it is never zero. `Slot` is `repr(C)`
  with `generation` first. `TaskSlab::entries()` (crate-private) gives `(TaskId, &T)` and stops once it
  has seen `len` live entries. `live_ids` and `build_report` use it.
  - After: [src/scope.rs L931-936 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L931-L936), `fire_expired_cancel_times` returning early on an empty
    heap.
  - After: [src/scope.rs L957-962 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L957-L962), `fire_expired_timers` returning early on an empty heap.
  - After: [src/scope.rs L978-988 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L978-L988), `next_deadline` returning `None` before the clock.
  - After: [src/scope.rs L620-623 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L620-L623), the poll loop: one `get_mut`.
  - After: [src/scope.rs L657-663 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L657-L663), the wake apply: one `get_mut`.
  - After: [src/scope.rs L968-974 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L968-L974), the timer pass: one `get_mut`.
  - After: [src/scope.rs L74-93 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L74-L93), `TaskEntry` as `repr(C)` with the per-round fields first.
  - After: [src/ids.rs L32-39 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/ids.rs#L32-L39), `Slot` as `repr(C)` with `generation` first.
  - After: [src/ids.rs L124-141 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/ids.rs#L124-L141), `live_ids` built on `entries()`, which stops after `len` live
    entries.
  - After: [src/scope.rs L990-991 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L990-L991), `build_report` through `entries()`.
  - Commit [c4fe22d](https://github.com/jameshuntt/scope/commit/c4fe22d20534104ed26b1311789f6cb7b622f21d).
  - Proof: [`proofs/entry-size`](proofs/entry-size/README) prints
    `TaskEntry 144 bytes, slab slot 152 bytes` at da655cf and `TaskEntry 136 bytes, slab slot 144 bytes`
    at c4fe22d, on aarch64-apple-darwin. Schedule golden passes at c4fe22d. Measured against da655cf,
    every bench moved between -4.3% and +3.6%, and we do not claim a speed change from it.
  - Regression, never published: with 1000 parked tasks, c4fe22d made the sparse probe (below) slower.
    In three runs each, da655cf took 1,419.5 to 1,440.0 µs and c4fe22d took 1,965.6 to 2,003.8 µs, 36.5%
    to 41.2% more. With 100 parked tasks c4fe22d took 252.2 to 253.7 µs against 243.9 to 246.8 µs, 2.2% to
    4.0% more. With 10 it was faster: 94.0 to 96.2 µs against 109.5 to 118.7 µs. 967b616 removes the
    per-round scan of every live task, and 0.3.1 does not show the slowdown (231.6 to 236.4 µs at 1000).
    We have not found which part of c4fe22d causes it. Not in 0.2.2; it came in with c4fe22d.
- **Runnable set: a round costs the runnable tasks, not every live one.** Each round called
  `order_runnable`. It built a `Vec` of every live task (`live_ids`), looked each one up to keep the
  runnable ones, and sorted those with comparators that looked both tasks up again on every comparison.
  The poll loop then looked each id up again. So a round cost every live task even when one was
  runnable.
  - Before: [src/scope.rs L613-627 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L613-L627), the poll loop over the ids `order_runnable` returns.
  - Before: [src/scope.rs L667-716 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L667-L716), `order_runnable`: `live_ids`, the filter, and sorts whose
    comparators read the slab.
  - Before, in 0.2.2: [src/scope.rs L638-687 @ d2b31b7](https://github.com/jameshuntt/scope/blob/d2b31b7a4536986df0a1c0fde9fe59ca67ffec7f/src/scope.rs#L638-L687), the same scan in `order_runnable`.

  Now `RunQueue` holds the runnable set. A task enters when its `runnable` flag goes true: at spawn, or
  when a wake is applied from the bus, which is how timer, finish-waiter and cancel wakes arrive. It
  leaves when the round takes it. Each round reads every member's key once into `(OrderKey, TaskId)`
  pairs and sorts them with no lookups in the comparator. The `TaskId` breaks ties in slot order, which
  is what the old stable sort over `live_ids` gave. `RoundRobin` rotates by the same cursor. Tasks past
  the poll budget stay queued with `runnable_seq` unchanged. A finished task's stale id is dropped at
  the next reorder.
  - After: [src/scope.rs L118-196 @ 967b616](https://github.com/jameshuntt/scope/blob/967b616129a566c1fc8eabf3148b1ed08b2159c6/src/scope.rs#L118-L196), `order_key` and `RunQueue`.
  - After: [src/scope.rs L689-716 @ 967b616](https://github.com/jameshuntt/scope/blob/967b616129a566c1fc8eabf3148b1ed08b2159c6/src/scope.rs#L689-L716), the round: `reorder`, then `take_next` up to the budget.
  - After: [src/scope.rs L741-754 @ 967b616](https://github.com/jameshuntt/scope/blob/967b616129a566c1fc8eabf3148b1ed08b2159c6/src/scope.rs#L741-L754), a wake applied from the bus entering the task.
  - After: [src/scope.rs L360 @ 967b616](https://github.com/jameshuntt/scope/blob/967b616129a566c1fc8eabf3148b1ed08b2159c6/src/scope.rs#L360), a spawn entering the task.
  - Commit [967b616](https://github.com/jameshuntt/scope/commit/967b616129a566c1fc8eabf3148b1ed08b2159c6).
  - Proof: the old full scan is kept under `cfg(test)` as `full_scan_order`, and in test builds every
    round asserts that its order equals it. `runnable_set_orders_like_a_full_scan` drives a seeded random
    workload under all 5 orderings and budgets `usize::MAX`, 1 and 3: spawns, wakes of live and finished
    tasks, timers, finish waiters, soft cancels that escalate by tick, and hard cancels between rounds.
    `cargo test --lib` passes it at 967b616 and at e9f73c3. Schedule golden passes at 967b616.
  - [src/scope.rs L1098-1141 @ 967b616](https://github.com/jameshuntt/scope/blob/967b616129a566c1fc8eabf3148b1ed08b2159c6/src/scope.rs#L1098-L1141): `full_scan_order`, the old order.
  - [src/scope.rs L691-704 @ 967b616](https://github.com/jameshuntt/scope/blob/967b616129a566c1fc8eabf3148b1ed08b2159c6/src/scope.rs#L691-L704): the assert in each round of a test build.
  - [src/scope.rs L1234-1284 @ 967b616](https://github.com/jameshuntt/scope/blob/967b616129a566c1fc8eabf3148b1ed08b2159c6/src/scope.rs#L1234-L1284): `runnable_set_orders_like_a_full_scan`.
  - Speed in the bench suite, against c4fe22d. The largest gains are `yield_storm` under
    `BiasedQoSThenAge`, whose old comparator did two to six slab lookups per comparison: 8.8%, 25.5% and
    31.8% less time at 10, 100 and 1000 tasks. Under `RoundRobin`, 7.4% to 12.4%. `sparse_wake_ring`,
    17.2% at 10 links and 16.4% at 100. Every other bench moved between -5.6% and +1.4%.
  - The case this commit is for, two runnable tasks among many parked ones, is not in the suite (see
    `sparse_wake_ring` under Added). [`proofs/sparse-probe`](proofs/sparse-probe/README) has two tasks
    passing 2000 wakes back and forth while 10, 100 or 1000 other tasks sit parked, then wakes the parked
    ones so the run finishes. Every version takes 1002 rounds. Each cell is the middle of three runs,
    and each run is the median of 101; all three runs are in the probe's README.

    | parked tasks | 0.2.2 | 0.3.0 | da655cf | c4fe22d | 967b616 | 0.3.1 |
    |---:|---:|---:|---:|---:|---:|---:|
    | 10 | 151.0 µs | 152.0 µs | 109.9 µs | 96.0 µs | 72.9 µs | 72.5 µs |
    | 100 | 269.9 µs | 265.2 µs | 244.0 µs | 253.5 µs | 88.2 µs | 88.2 µs |
    | 1000 | 1,373.6 µs | 1,368.9 µs | 1,429.8 µs | 1,967.8 µs | 233.9 µs | 235.3 µs |

    From 0.3.0 to 0.3.1 that is 52.3%, 66.7% and 82.8% less time.
  - In 0.2.2: `cd proofs/sparse-probe/0.2.2 && cargo run --release` gave
    `idle=1000 rounds=1002 polls=4002 completed=1002 median_us=1373.5`. The same command in
    `proofs/sparse-probe/0.3.1` gave `median_us=231.6` (the first of three runs each).

The README's 0.3.1 section and the version bump are in [e9f73c3](https://github.com/jameshuntt/scope/commit/e9f73c31020cd59736b8b59c47a930f153cf7063).

### Fixed

- **`run()` and `run_until()` lost a wake that arrived while they waited.** When a round polled nothing
  and a timer or cancel deadline was pending, `run()` waited on the bus with `recv_timeout(wait)`. That
  call takes the message that ended the wait off the queue, and `run()` threw it away (`let _ = ...`). A
  wake from another thread, from a real `Waker` or a `MailboxSender::send`, that landed during the wait
  never reached its task. The task stayed parked, the run slept on to the deadline, and the task came
  back in `RunReport::stalled`. `run_until()` waits the same way whenever a round polls nothing, up to
  its own timeout even with no deadline pending, and it dropped the wake the same way.
  - Before: [src/scope.rs L556-579 @ 513cb50](https://github.com/jameshuntt/scope/blob/513cb506a7cbec53351d27459db014224f2abef8/src/scope.rs#L556-L579), `run()` with `let _ = self.bus.recv_timeout(wait);`.
  - Before: [src/scope.rs L584-606 @ 513cb50](https://github.com/jameshuntt/scope/blob/513cb506a7cbec53351d27459db014224f2abef8/src/scope.rs#L584-L606), `run_until()` with the same line.
  - Before: [src/wake.rs L170-178 @ 513cb50](https://github.com/jameshuntt/scope/blob/513cb506a7cbec53351d27459db014224f2abef8/src/wake.rs#L170-L178), `recv_timeout` taking the message off the queue.
  - Before, in 0.3.0: [src/scope.rs L530-580 @ 6fce0fe](https://github.com/jameshuntt/scope/blob/6fce0feab3bac3e107f9cff0763cf92dc5922659/src/scope.rs#L530-L580), `run()` and `run_until()` with the same two
    lines.
  - Before, in 0.3.0: [src/wake.rs L123-136 @ 6fce0fe](https://github.com/jameshuntt/scope/blob/6fce0feab3bac3e107f9cff0763cf92dc5922659/src/wake.rs#L123-L136), `recv_timeout` taking the message off the queue.

  Now the idle wait is `WakeBus::wait_timeout` (crate-private). It takes one lock, returns at once if
  anything is queued, and leaves the message on the bus for the next round's `drain_bus`. With no
  deadline, `run()` still ends when the bus is empty. `recv_timeout` keeps its public behaviour.
  - After: [src/scope.rs L556-577 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L556-L577), `run()` waiting with `wait_timeout`.
  - After: [src/scope.rs L582-602 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L582-L602), `run_until()` waiting with `wait_timeout`.
  - After: [src/wake.rs L170-175 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/wake.rs#L170-L175), `wait_timeout`, which never takes the message.
  - Commit [da655cf](https://github.com/jameshuntt/scope/commit/da655cf5434a7a33d427615f6dd89505781b4d9a).
  - Proof: `a_wake_from_another_thread_during_runs_idle_wait_is_not_lost`, [tests/kernel.rs L219-258 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/tests/kernel.rs#L219-L258).
    A task parks and hands its real waker to a thread that fires it after 50 ms, while `run()` waits on
    a 10 s sleep. Once woken, the task cancels the sleep. [`proofs/lost-wake`](proofs/lost-wake/README)
    runs this test, and the same check through `run_until()`, against 0.2.2, 0.3.0, 513cb50, da655cf
    and 0.3.1. On 0.2.2, 0.3.0 and 513cb50 both tests fail. On da655cf and 0.3.1 both pass, in 0.06 s.
  - In 0.2.2: `cd proofs/lost-wake/0.2.2 && cargo test` gave
    `RunReport { rounds: 5, polls: 3, completed: 1, cancelled: 0, panicked: 0, stalled: [(TaskId { slot: 1, generation: 0 }, "woken")] }`
    for `run()` and `finished=false after 3.004183667s` for `run_until()`, then
    `test result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 10.00s`.
  - Where it came from: it was already there in 0.2.0. The two `let _ = self.bus.recv_timeout(wait);`
    lines are unchanged from 0.2.0 through 0.3.0, so every published 0.2 and 0.3.0 has it. c8c063e is
    the commit that matches the published 0.2.0 package: the package's `Cargo.toml.orig` equals its
    `Cargo.toml`, and every other file in the package but the generated `Cargo.toml` and
    `.cargo_vcs_info.json` is the same byte for byte.
  - [src/scope.rs L545 @ c8c063e](https://github.com/jameshuntt/scope/blob/c8c063e50d9cfc661ad64ea15f2daa0da85223a1/src/scope.rs#L545): the line in `run()` in 0.2.0.
  - [src/scope.rs L579 @ c8c063e](https://github.com/jameshuntt/scope/blob/c8c063e50d9cfc661ad64ea15f2daa0da85223a1/src/scope.rs#L579): the line in `run_until()` in 0.2.0.
- **The `run_scoped` doc test that must not compile now fails for the borrow and nothing else.** The
  example was a `compile_fail` test whose closure had no type on its argument (`move |_cx|`). rustc
  rejects that closure on its own, with "implementation of `FnOnce` is not general enough" and the same
  for `FnMut`, whatever it borrows and with no borrow at all. So the test passed for the wrong reason, and it would have kept
  passing with a legal borrow.
  - Before: [src/scope.rs L248-263 @ 407684b](https://github.com/jameshuntt/scope/blob/407684bdc7f1bcc360afaf38acf4783f72b3484a/src/scope.rs#L248-L263), the old `compile_fail` example with `move |_cx|`.

  Now there are two examples that differ only in where the borrowed `String` lives. The first borrows
  data that outlives the scope and has to compile. The second borrows data that dies inside the
  `run_scoped` closure. It is marked `compile_fail,E0597` and types its closure argument
  (`|_cx: &mut Cx<'_, '_, ()>|`), so the only error left is the borrow.
  - After: [src/scope.rs L244-283 @ e9f73c3](https://github.com/jameshuntt/scope/blob/e9f73c31020cd59736b8b59c47a930f153cf7063/src/scope.rs#L244-L283), the two examples.
  - Commit [e9f73c3](https://github.com/jameshuntt/scope/commit/e9f73c31020cd59736b8b59c47a930f153cf7063).
  - Proof: [`proofs/borrow-doctest`](proofs/borrow-doctest/README), pinned to 0.2.2, builds each
    closure on its own. `cargo build --example as_written`, the old example, gives 5 errors: E0597 and
    four "not general enough". `cargo build --example legal_untyped`, the same code with the `String`
    moved out of the scope, gives the four "not general enough" errors and no E0597.
    `cargo build --example no_borrow_untyped`, a task that borrows nothing, gives the same four.
    `cargo build --example illegal_typed`, the new second example, gives one error, E0597.
    `cargo run --example legal_typed`, the new first example, prints `completed=1`. At e9f73c3,
    `cargo test --doc run_scoped` passes both examples, on stable and on nightly.
  - Stable rustdoc does not check the `E0597` code on a `compile_fail` test; nightly does.
    [`proofs/doctest-error-code`](proofs/doctest-error-code/README) marks the new example with the
    wrong code, E0308. Stable passes it and nightly fails it with
    `Some expected error codes were not found: ["E0308"]`. The CI test job runs doc tests on stable, so
    CI checks that one example compiles and the other does not, but not the code.
  - In 0.2.2: `cd proofs/borrow-doctest && cargo test --doc` gave
    `test src/lib.rs - legal_borrow_untyped_closure (line 28) - compile fail ... ok`. That is a legal
    borrow, written the way the 0.2.2 test writes its closure, passing as `compile_fail`.
  - Where it came from: it was already there in 0.2.0. [src/scope.rs L142-157 @ c8c063e](https://github.com/jameshuntt/scope/blob/c8c063e50d9cfc661ad64ea15f2daa0da85223a1/src/scope.rs#L142-L157) is the same
    example in 0.2.0.
- **The loom build of the lib tests failed. Never published.** 967b616 added a `#[cfg(test)] mod tests`
  to `scope.rs` that builds a whole `Scope`. Under `--cfg loom` that module compiled too, and its test
  touched loom's `Mutex` outside a `loom::model`. So `RUSTFLAGS="--cfg loom" cargo test --lib --release`,
  the line 513cb50 added to `verify.sh`, failed with "cannot access Loom execution state from outside a
  Loom model". 407684b gates the module with `#[cfg(all(test, not(loom)))]`.
  - Before: [src/scope.rs L1143-1144 @ 967b616](https://github.com/jameshuntt/scope/blob/967b616129a566c1fc8eabf3148b1ed08b2159c6/src/scope.rs#L1143-L1144), the module under plain `#[cfg(test)]`.
  - After: [src/scope.rs L1143-1145 @ 407684b](https://github.com/jameshuntt/scope/blob/407684bdc7f1bcc360afaf38acf4783f72b3484a/src/scope.rs#L1143-L1145), the module under `#[cfg(all(test, not(loom)))]`.
  - Commit [407684b](https://github.com/jameshuntt/scope/commit/407684bdc7f1bcc360afaf38acf4783f72b3484a).
  - Not in 0.2.2; that module did not exist. It came in with 967b616, part of 0.3.1 itself. 407684b
    landed before 0.3.1 was published, so no published version has it.
  - Proof: we ran that command at both, each in its own fresh target dir. At 967b616: 4 passed, 1 failed
    (`runnable_set_orders_like_a_full_scan`, with the message above). At 407684b: 4 passed, 0 failed (the
    two `ids` tests and both loom models).

## 0.3.0 (2026-10-15 04:43 UTC)

Published from [6fce0fe](https://github.com/jameshuntt/scope/commit/6fce0feab3bac3e107f9cff0763cf92dc5922659). The package's `.cargo_vcs_info.json` names that commit.

### Changed

- **On `waker_tables` 0.3. The waker's vtable is built from `WakeSignal` itself.** 0.2.x built its waker
  with `waker_tables` 0.1.4. A private `vtable` module declared `WAKE_SIGNAL_VTABLE` with
  `declare_raw_waker_vtable!`, and `scheduler_waker` passed it in as
  `waker_from_arc(arc, &WAKE_SIGNAL_VTABLE)`. In 0.1.4 `waker_from_arc` is a safe fn that takes any
  `&'static RawWakerVTable` (line 162 of `src/lib.rs` in the 0.1.4 package), so nothing tied the vtable
  to the type. In 0.3 it is `waker_from_arc(arc)`, and the vtable comes from `T`. The `vtable` module and
  `WAKE_SIGNAL_VTABLE` are gone.
  - Before: [src/wake.rs L161-179 @ bf2b88f](https://github.com/jameshuntt/scope/blob/bf2b88fcaa14194cc2f24dd2e6d793830e2da0e1/src/wake.rs#L161-L179), the `vtable` module and `scheduler_waker` passing
    `&WAKE_SIGNAL_VTABLE`.
  - Before: [Cargo.toml L17 @ bf2b88f](https://github.com/jameshuntt/scope/blob/bf2b88fcaa14194cc2f24dd2e6d793830e2da0e1/Cargo.toml#L17), `waker_tables = "0.1.4"`.
  - After: [src/wake.rs L162-171 @ 6fce0fe](https://github.com/jameshuntt/scope/blob/6fce0feab3bac3e107f9cff0763cf92dc5922659/src/wake.rs#L162-L171), `scheduler_waker` calling `waker_from_arc(arc)`.
  - After: [Cargo.toml L17 @ 6fce0fe](https://github.com/jameshuntt/scope/blob/6fce0feab3bac3e107f9cff0763cf92dc5922659/Cargo.toml#L17), `waker_tables = "0.3"`.
  - README note: [README.md L155-160 @ 6fce0fe](https://github.com/jameshuntt/scope/blob/6fce0feab3bac3e107f9cff0763cf92dc5922659/README.md#L155-L160), the README's 0.3.0 section.
  - Commit [6fce0fe](https://github.com/jameshuntt/scope/commit/6fce0feab3bac3e107f9cff0763cf92dc5922659).
  - This was not a soundness hole in scope. `scheduler_waker` always passed `WakeSignal`'s own vtable. It
    never takes one from a caller, and scope does not re-export `waker_tables`. The version is minor
    because `WakeSignal` is public and implements `waker_tables`' public trait, which changed version.
    The README says so.
  - Proof: `real_waker_routes_back_to_bus`, [src/wake.rs L189-201 @ 6fce0fe](https://github.com/jameshuntt/scope/blob/6fce0feab3bac3e107f9cff0763cf92dc5922659/src/wake.rs#L189-L201). A clone and the original
    both wake the same task, through the vtable's clone, wake, wake_by_ref and drop.
  - Proof: `a_real_external_waker_resumes_a_parked_task`, [tests/kernel.rs L199-217 @ 6fce0fe](https://github.com/jameshuntt/scope/blob/6fce0feab3bac3e107f9cff0763cf92dc5922659/tests/kernel.rs#L199-L217). A task
    parks, and a real `Waker` from `waker_for` brings it back.
  - Both pass at 6fce0fe with the rest of `cargo test` (lib 4, kernel 13, mailbox 3, doc 3).
    `cargo +nightly miri test --lib` at 6fce0fe passes too, `real_waker_routes_back_to_bus` included.
    CI runs the lib tests under Miri on pushes to `main`, on `v*` tags, on pull requests and once a week.
    Run 38692079734 is on the 0.3.1 code, whose waker code is the same.

### Fixed

- **`rust-version` is 1.85. It said 1.80, which could not build.** 0.2.0 through 0.2.3 declared
  `rust-version = "1.80"`. Their dependency `waker_tables` 0.1.4 is edition 2024, and edition 2024 needs
  Rust 1.85.
  - Before: [Cargo.toml L5 @ bf2b88f](https://github.com/jameshuntt/scope/blob/bf2b88fcaa14194cc2f24dd2e6d793830e2da0e1/Cargo.toml#L5), `rust-version = "1.80"`.
  - After: [Cargo.toml L5 @ 6fce0fe](https://github.com/jameshuntt/scope/blob/6fce0feab3bac3e107f9cff0763cf92dc5922659/Cargo.toml#L5), `rust-version = "1.85"`.
  - Commit [6fce0fe](https://github.com/jameshuntt/scope/commit/6fce0feab3bac3e107f9cff0763cf92dc5922659).
  - Proof: the CI msrv job builds and tests on 1.85 (run 38692079734).
  - In 0.2.2: `cd proofs/rust-version && cargo +1.80 check --locked` gave
    `failed to parse manifest at .../waker_tables-0.1.4/Cargo.toml` and `feature edition2024 is required`,
    exit 101. `cargo +1.85 check --locked` builds.
  - Where it came from: it was already there in 0.2.0. [Cargo.toml L5 @ c8c063e](https://github.com/jameshuntt/scope/blob/c8c063e50d9cfc661ad64ea15f2daa0da85223a1/Cargo.toml#L5) is the same line in
    0.2.0.

## 0.2.3 (2026-10-15 00:52 UTC)

Published from a copy of the 0.2.2 package, not from a git checkout. The `.crate` has no
`.cargo_vcs_info.json`. Its `Cargo.toml.orig` is the 0.2.2 package's normalized manifest with the
version changed, and its SHA-256 matches the crates.io checksum (`f19c3edd...`). Every file besides the
two manifests matches [bf2b88f](https://github.com/jameshuntt/scope/commit/bf2b88fcaa14194cc2f24dd2e6d793830e2da0e1) byte for byte.

### Changed

- **The Apache license names its copyright holder.** `LICENSE-APACHE` still had the template line
  `Copyright [yyyy] [name of copyright owner]`. It now reads `Copyright 2026 1o LLC`. No code changed.
  - Before: [LICENSE-APACHE L190 @ d2b31b7](https://github.com/jameshuntt/scope/blob/d2b31b7a4536986df0a1c0fde9fe59ca67ffec7f/LICENSE-APACHE#L190), the template line.
  - After: [LICENSE-APACHE L190 @ bf2b88f](https://github.com/jameshuntt/scope/blob/bf2b88fcaa14194cc2f24dd2e6d793830e2da0e1/LICENSE-APACHE#L190), the line with the holder's name.
  - Commit [bf2b88f](https://github.com/jameshuntt/scope/commit/bf2b88fcaa14194cc2f24dd2e6d793830e2da0e1).

## 0.2.2 (2026-09-03 01:55 UTC)

Published from another repository. The package's `.cargo_vcs_info.json` says commit
`aca125861ea95ad6546f1f428cde4c9c4278f941`, path `rust/published/scope`. [d2b31b7](https://github.com/jameshuntt/scope/commit/d2b31b7a4536986df0a1c0fde9fe59ca67ffec7f) in this repo has
the same files: `src/`, `tests/`, `examples/`, `README.md`, the licenses, `Cargo.lock`, and a
`Cargo.toml` equal to the package's `Cargo.toml.orig`.

### Added

- **README: "When to use it".** When scope fits: many small cooperative steps sharing one piece of
  mutable state, with the same schedule every run. And how it differs from an async executor,
  `std::thread::scope` and rayon's `scope`, and a hand-rolled poll loop. No code changed from 0.2.1.
  - [README.md L16-51 @ d2b31b7](https://github.com/jameshuntt/scope/blob/d2b31b7a4536986df0a1c0fde9fe59ca67ffec7f/README.md#L16-L51): the new section.

### Known problems, fixed later

- `run()` and `run_until()` drop a wake that ends their idle wait. Fixed in 0.3.1.
  - [src/scope.rs L536-549 @ d2b31b7](https://github.com/jameshuntt/scope/blob/d2b31b7a4536986df0a1c0fde9fe59ca67ffec7f/src/scope.rs#L536-L549): the wait in `run()`.
  - [src/scope.rs L565-578 @ d2b31b7](https://github.com/jameshuntt/scope/blob/d2b31b7a4536986df0a1c0fde9fe59ca67ffec7f/src/scope.rs#L565-L578): the wait in `run_until()`.
- The `run_scoped` `compile_fail` example also fails for a closure error that has nothing to do with the
  borrow, so it would pass with a legal borrow too. Fixed in 0.3.1.
  - [src/scope.rs L140-155 @ d2b31b7](https://github.com/jameshuntt/scope/blob/d2b31b7a4536986df0a1c0fde9fe59ca67ffec7f/src/scope.rs#L140-L155): the example.
- `rust-version = "1.80"` while `waker_tables` 0.1.4 needs 1.85. Fixed in 0.3.0.
  - [Cargo.toml L5 @ d2b31b7](https://github.com/jameshuntt/scope/blob/d2b31b7a4536986df0a1c0fde9fe59ca67ffec7f/Cargo.toml#L5): the line.
- Every round scans every live task to find the runnable ones. A cost, not a bug. Changed in 0.3.1 by
  the runnable set.
  - [src/scope.rs L638-687 @ d2b31b7](https://github.com/jameshuntt/scope/blob/d2b31b7a4536986df0a1c0fde9fe59ca67ffec7f/src/scope.rs#L638-L687): `order_runnable`.
