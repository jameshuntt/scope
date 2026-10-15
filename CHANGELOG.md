# Changelog

Every code link below is pinned to a commit, so it opens the exact lines as they were at that commit. "Before" links
show the code that had the problem, "after" links show the change. Publish times come from the crates.io API, in UTC.

## Unreleased

### Added

- **CI.** `.github/workflows/ci.yml` runs on every push to `main`, on pull requests and once a week. It runs the tests
  (the pinned schedule included), clippy and rustdoc with `-D warnings`, `cargo bench --no-run`, `cargo package`, the
  tests and clippy on an ARM runner (the 1BRC example's NEON code only builds on aarch64), the tests on Rust 1.85,
  Miri on the lib and kernel tests, and the loom tests. A `v*` tag publishes to crates.io only after the test, arm,
  msrv, miri and loom jobs pass, and only if the tag matches the version in `Cargo.toml`.
  [.github/workflows/ci.yml L1-117 @ 782814c](https://github.com/jameshuntt/scope/blob/782814cc9bd7b5cb8a0746d46113e7e6da7c8f34/.github/workflows/ci.yml#L1-L117), README note [README.md L148-150 @ 782814c](https://github.com/jameshuntt/scope/blob/782814cc9bd7b5cb8a0746d46113e7e6da7c8f34/README.md#L148-L150), commit [782814c](https://github.com/jameshuntt/scope/commit/782814cc9bd7b5cb8a0746d46113e7e6da7c8f34).
  - Proof: run [38692079734](https://github.com/jameshuntt/scope/actions/runs/38692079734) on 782814c passed every
    job (test, arm, msrv, miri, loom, latest-deps on stable and on 1.85). The publish job was skipped because the push
    was not a tag.

## 0.3.1 (2026-10-15 06:17 UTC)

Published from [e9f73c3](https://github.com/jameshuntt/scope/commit/e9f73c31020cd59736b8b59c47a930f153cf7063); the package's `.cargo_vcs_info.json` names that commit. The commits in it, oldest
first: 19b484b, cfb873a, 513cb50, da655cf, c4fe22d, 967b616, 407684b, e9f73c3.

**The borrow is unchanged.** `ScopedTask<'env, R>` and `Cx`'s `pub resources: &'poll mut R` read the same as in 0.3.0
([src/task.rs L64-74 @ 6fce0fe](https://github.com/jameshuntt/scope/blob/6fce0feab3bac3e107f9cff0763cf92dc5922659/src/task.rs#L64-L74), [src/task.rs L63-73 @ e9f73c3](https://github.com/jameshuntt/scope/blob/e9f73c31020cd59736b8b59c47a930f153cf7063/src/task.rs#L63-L73)). The only change inside `Cx` is crate-private: `bus`
and `cancel` went from owned values (`WakeBus`, `Arc<CancelCell>`) to borrows for the poll (`&'poll WakeBus`,
`&'poll CancelCell`), [src/task.rs L77-78 @ 6fce0fe](https://github.com/jameshuntt/scope/blob/6fce0feab3bac3e107f9cff0763cf92dc5922659/src/task.rs#L77-L78) to [src/task.rs L76-79 @ e9f73c3](https://github.com/jameshuntt/scope/blob/e9f73c31020cd59736b8b59c47a930f153cf7063/src/task.rs#L76-L79). We checked it three ways:

- No line declaring a public item changed. `git diff 6fce0fe e9f73c3 -- src`, filtered to lines with `pub` and
  without `pub(crate)`, shows only `WakeBus::recv_timeout` moving down the file with the same signature.
- The borrow tests pass on 0.3.1: `scoped_borrow_lets_many_tasks_mutate_env` [tests/kernel.rs L27-39 @ e9f73c3](https://github.com/jameshuntt/scope/blob/e9f73c31020cd59736b8b59c47a930f153cf7063/tests/kernel.rs#L27-L39)
  (three tasks share `&mut u64`, no lock) and `tasks_hold_distinct_mut_borrows_of_stack_data_across_suspension`
  [tests/kernel.rs L326-365 @ e9f73c3](https://github.com/jameshuntt/scope/blob/e9f73c31020cd59736b8b59c47a930f153cf7063/tests/kernel.rs#L326-L365) (each task holds its own `&'env mut Vec` across yields while they interleave).
- The `run_scoped` doc examples now pin the bound from both sides: a borrow of data that outlives the scope compiles,
  the same task borrowing data that dies first fails with E0597 and nothing else. See Fixed below.

No `unsafe` line in `src/` changed from 0.2.2 to 0.3.1: `git diff d2b31b7 782814c -- src | grep unsafe` matches one
doc comment and nothing else. Everything outside `wake` is still `#![deny(unsafe_code)]`.

### Added

- **The exact schedule is pinned.** `tests/schedule_golden.rs` runs 4 workloads (yielders, a wake ring, spawn and
  join, cancel with tick escalation) under all 5 `WakeOrdering`s and poll budgets `usize::MAX`, 3 and 1, once with a
  tracer recording every hook and once with the tasks logging their own polls. The output has to equal
  `tests/golden/schedule.txt` (120 runs, 10,631 lines), recorded from 0.3.0: 19b484b's `src/` is identical to
  6fce0fe's. Any change to who is polled when, how many rounds a run takes, or what the tracer sees fails it.
  [tests/schedule_golden.rs L1-7 @ 19b484b](https://github.com/jameshuntt/scope/blob/19b484b84cff1f4853ce89602ddb5b5822bc8b67/tests/schedule_golden.rs#L1-L7), [tests/schedule_golden.rs L231-264 @ 19b484b](https://github.com/jameshuntt/scope/blob/19b484b84cff1f4853ce89602ddb5b5822bc8b67/tests/schedule_golden.rs#L231-L264), commit [19b484b](https://github.com/jameshuntt/scope/commit/19b484b84cff1f4853ce89602ddb5b5822bc8b67).
  - Proof: it passes at 19b484b and at every commit after it. We ran `cargo test --test schedule_golden` at each of
    19b484b, cfb873a, 513cb50, da655cf, c4fe22d, 967b616, 407684b and e9f73c3, touching the sources first so each one
    rebuilt. It also catches a change: giving `WakeOrdering::Lifo` the FIFO key in `order_key`
    ([src/scope.rs L136 @ e9f73c3](https://github.com/jameshuntt/scope/blob/e9f73c31020cd59736b8b59c47a930f153cf7063/src/scope.rs#L136)) makes it fail at line 753, `now Some("poll 1 #0.0 y0"), recorded Some("poll 1 #11.0 y11")`.
- **Scheduler benchmarks.** `benches/scheduler.rs` (criterion): `yield_storm` (10, 100 and 1000 tasks yielding 20
  times under `BiasedQoSThenAge`, `RoundRobin` and `Fifo`), `sparse_wake_ring` (a ring of 10, 100 or 1000 links passing
  2000 wakes), `spawn_churn`, `join_all` and `mailbox`. Each iteration builds the scope, spawns and runs to
  completion, so spawn and finish are in the number. `cargo bench --bench scheduler`. The benches are in the repo,
  not in the published package (`include` in `Cargo.toml` has no `benches/**`).
  [benches/scheduler.rs L1-200 @ 19b484b](https://github.com/jameshuntt/scope/blob/19b484b84cff1f4853ce89602ddb5b5822bc8b67/benches/scheduler.rs#L1-L200), [Cargo.toml L27-31 @ 19b484b](https://github.com/jameshuntt/scope/blob/19b484b84cff1f4853ce89602ddb5b5822bc8b67/Cargo.toml#L27-L31), commit [19b484b](https://github.com/jameshuntt/scope/commit/19b484b84cff1f4853ce89602ddb5b5822bc8b67).
  - Problem, not fixed yet: `sparse_wake_ring` is not sparse. Its comment says only one link is runnable at a time
    [benches/scheduler.rs L25-26 @ 19b484b](https://github.com/jameshuntt/scope/blob/19b484b84cff1f4853ce89602ddb5b5822bc8b67/benches/scheduler.rs#L25-L26), but every link starts runnable and each poll wakes the next link, so
    every link is runnable in every round. We ran the same workload and counted: 10 links take 201 rounds and 2010
    polls, 100 links take 21 rounds and 2100 polls, 1000 links take 3 rounds and 3000 polls. The README's "wake-driven
    work among many idle tasks" [README.md L158-159 @ e9f73c3](https://github.com/jameshuntt/scope/blob/e9f73c31020cd59736b8b59c47a930f153cf7063/README.md#L158-L159) is this bench, so that label is wrong too. Its numbers
    are for a ring where every task is runnable. This came in with 19b484b, part of 0.3.1. The case the comment
    describes is measured with a separate probe, under the runnable set entry below.

### Changed

The scheduler is faster with the same schedule. The README states the 0.3.0 to 0.3.1 numbers
([README.md L155-168 @ e9f73c3](https://github.com/jameshuntt/scope/blob/e9f73c31020cd59736b8b59c47a930f153cf7063/README.md#L155-L168)). We ran `cargo bench --bench scheduler` again for this changelog, once at every
commit and in commit order, on one Apple Silicon Mac (aarch64-apple-darwin, rustc 1.97.1, criterion defaults, other
work running). 0.3.0 is 19b484b (same `src/` as 6fce0fe, plus the bench) and 0.3.1 is e9f73c3. Time per iteration,
criterion's point estimate:

| bench | 0.3.0 | 0.3.1 | change |
|---|---:|---:|---:|
| `yield_storm/biased_qos_then_age/10` | 12.4 µs | 8.1 µs | -34.6% |
| `yield_storm/biased_qos_then_age/100` | 121.0 µs | 73.2 µs | -39.5% |
| `yield_storm/biased_qos_then_age/1000` | 1.339 ms | 775.9 µs | -42.1% |
| `yield_storm/round_robin/10` | 12.2 µs | 8.4 µs | -31.0% |
| `yield_storm/round_robin/100` | 107.3 µs | 73.9 µs | -31.1% |
| `yield_storm/round_robin/1000` | 1.009 ms | 784.8 µs | -22.2% |
| `yield_storm/fifo/10` | 11.6 µs | 8.0 µs | -31.0% |
| `yield_storm/fifo/100` | 96.6 µs | 72.0 µs | -25.5% |
| `yield_storm/fifo/1000` | 1.017 ms | 780.3 µs | -23.2% |
| `sparse_wake_ring/10` | 103.6 µs | 62.2 µs | -40.0% |
| `sparse_wake_ring/100` | 113.4 µs | 72.5 µs | -36.0% |
| `sparse_wake_ring/1000` | 263.6 µs | 197.4 µs | -25.1% |
| `spawn_churn/100` | 15.0 µs | 11.6 µs | -22.8% |
| `spawn_churn/1000` | 140.1 µs | 99.5 µs | -28.9% |
| `spawn_churn/10000` | 1.382 ms | 1.073 ms | -22.4% |
| `join_all/10` | 5.0 µs | 4.2 µs | -15.4% |
| `join_all/100` | 39.0 µs | 34.5 µs | -11.5% |
| `join_all/1000` | 383.0 µs | 328.4 µs | -14.2% |
| `mailbox/100` | 3.0 µs | 2.5 µs | -14.8% |
| `mailbox/10000` | 207.1 µs | 172.9 µs | -16.5% |

That lands close to the README: `yield_storm` under the default `BiasedQoSThenAge` 34.6% to 42.1% less time (README
33% to 42%), `sparse_wake_ring` 25.1% to 40.0% (README 25% to 41%), `spawn_churn` 22.4% to 28.9% (README 22% to 27%),
`join_all` and `mailbox` 11.5% to 16.5% (README 12% to 16%).

How much of that is noise: 19b484b run a second time at the end, 23 minutes after the first, moved by -3.7% to
+18.1% per bench, and 967b616 against e9f73c3, which have the same code outside tests and docs, moved by -2.0% to
+4.6%. So a single per-commit number under about 10% below is not proof of anything on its own.

The speed came from four commits. Each entry gives its own lines and the change it made on its own, measured against
the commit before it.

- **Poll path: no name clone, no `Arc` traffic, reused intent buffers.** Every poll cloned the task's name
  (`e.name.clone()`, a `String` allocation) and its cancel cell's `Arc`, and built `Cx` with another clone of the bus
  `Arc`. Both `Arc`s were dropped again when the poll ended. It also made four new `Vec`s for the intents, so a poll
  that spawned, slept, subscribed or cancelled allocated them fresh.
  Before: [src/scope.rs L695-701 @ 19b484b](https://github.com/jameshuntt/scope/blob/19b484b84cff1f4853ce89602ddb5b5822bc8b67/src/scope.rs#L695-L701) (the clones), [src/scope.rs L716-721 @ 19b484b](https://github.com/jameshuntt/scope/blob/19b484b84cff1f4853ce89602ddb5b5822bc8b67/src/scope.rs#L716-L721) (fresh `Vec`s),
  [src/scope.rs L749-759 @ 19b484b](https://github.com/jameshuntt/scope/blob/19b484b84cff1f4853ce89602ddb5b5822bc8b67/src/scope.rs#L749-L759) (`Cx` with `bus.clone()` and `cell.clone()`), [src/task.rs L77-78 @ 19b484b](https://github.com/jameshuntt/scope/blob/19b484b84cff1f4853ce89602ddb5b5822bc8b67/src/task.rs#L77-L78).
  Now `Cx` borrows the bus and the cell for the poll, the tracer borrows the name from the slab entry and only when a
  tracer is attached, and the four buffers live in the `Scope` as `PollIntents` and keep their capacity.
  `insert_task` moves the name into the entry instead of cloning it, and a child spawned mid-poll gets its cancel cell
  when the spawn is applied instead of inside the hook.
  After: [src/task.rs L76-79 @ cfb873a](https://github.com/jameshuntt/scope/blob/cfb873ab14af169304d80b81bac2f75211ec8bb0/src/task.rs#L76-L79), [src/scope.rs L94-109 @ cfb873a](https://github.com/jameshuntt/scope/blob/cfb873ab14af169304d80b81bac2f75211ec8bb0/src/scope.rs#L94-L109), [src/scope.rs L717-748 @ cfb873a](https://github.com/jameshuntt/scope/blob/cfb873ab14af169304d80b81bac2f75211ec8bb0/src/scope.rs#L717-L748),
  [src/scope.rs L773-790 @ cfb873a](https://github.com/jameshuntt/scope/blob/cfb873ab14af169304d80b81bac2f75211ec8bb0/src/scope.rs#L773-L790), [src/scope.rs L259-278 @ cfb873a](https://github.com/jameshuntt/scope/blob/cfb873ab14af169304d80b81bac2f75211ec8bb0/src/scope.rs#L259-L278), [src/scope.rs L811 @ cfb873a](https://github.com/jameshuntt/scope/blob/cfb873ab14af169304d80b81bac2f75211ec8bb0/src/scope.rs#L811). Commit [cfb873a](https://github.com/jameshuntt/scope/commit/cfb873ab14af169304d80b81bac2f75211ec8bb0).
  The entry the cell is borrowed from stays in the slab for the whole poll because the slab is borrowed shared for that
  long, and the compiler checks it; `scope.rs` has no `unsafe`.
  - Proof: schedule golden passes at cfb873a. Measured against 19b484b: `yield_storm` 13.5% to
    21.4% less time, `sparse_wake_ring` 19.3% to 22.2%, `spawn_churn` 19.3% to 23.5%, `join_all` 12.6% to 14.7%.
    `mailbox` moved 2.0% and 2.9%, inside the noise.
  - c4fe22d's commit message also lists the `insert_task` name move. The line changed here, in cfb873a.
- **Wake bus: one lock per drain, no notify when nobody waits.** `drain_bus` took wakes off the bus one `try_recv` at a
  time, so N queued wakes cost N + 1 locks of the bus mutex. `send` called `notify_one` on every send, with or without
  a receiver blocked on the condvar.
  Before: [src/scope.rs L640-658 @ cfb873a](https://github.com/jameshuntt/scope/blob/cfb873ab14af169304d80b81bac2f75211ec8bb0/src/scope.rs#L640-L658), [src/wake.rs L97-115 @ cfb873a](https://github.com/jameshuntt/scope/blob/cfb873ab14af169304d80b81bac2f75211ec8bb0/src/wake.rs#L97-L115).
  Now `drain_bus` swaps its empty `VecDeque` (`wake_batch`) for the bus queue under one lock and applies the batch with
  the lock released, oldest first, as before. With a tracer attached it takes the queue once more after applying,
  because a tracer hook is the only code on the scheduler thread that can send while a batch is applied; that keeps
  those wakes in the same round, as one-at-a-time popping did. With no tracer, a wake another thread sends after the
  swap waits for the next round. `send` calls `notify_one` only when `Queue::waiting` is above zero. A receiver counts
  itself under the lock before its wait releases the lock, so a sender that reads 0 has nobody to wake.
  After: [src/scope.rs L644-684 @ 513cb50](https://github.com/jameshuntt/scope/blob/513cb506a7cbec53351d27459db014224f2abef8/src/scope.rs#L644-L684), [src/wake.rs L69-77 @ 513cb50](https://github.com/jameshuntt/scope/blob/513cb506a7cbec53351d27459db014224f2abef8/src/wake.rs#L69-L77), [src/wake.rs L110-118 @ 513cb50](https://github.com/jameshuntt/scope/blob/513cb506a7cbec53351d27459db014224f2abef8/src/wake.rs#L110-L118),
  [src/wake.rs L138-168 @ 513cb50](https://github.com/jameshuntt/scope/blob/513cb506a7cbec53351d27459db014224f2abef8/src/wake.rs#L138-L168). Commit [513cb50](https://github.com/jameshuntt/scope/commit/513cb506a7cbec53351d27459db014224f2abef8).
  - Proof: unit tests `take_all_keeps_fifo_order_behind_leftovers` and
    `a_blocked_receiver_is_woken_by_a_send_from_another_thread` [src/wake.rs L245-290 @ 513cb50](https://github.com/jameshuntt/scope/blob/513cb506a7cbec53351d27459db014224f2abef8/src/wake.rs#L245-L290). Two loom models, a
    blocked receiver against one sender and against two [src/wake.rs L293-346 @ 513cb50](https://github.com/jameshuntt/scope/blob/513cb506a7cbec53351d27459db014224f2abef8/src/wake.rs#L293-L346). loom's `wait_timeout` never
    times out, so a skipped notify shows up as a deadlock. `verify.sh` runs them [verify.sh L21 @ 513cb50](https://github.com/jameshuntt/scope/blob/513cb506a7cbec53351d27459db014224f2abef8/verify.sh#L21), and so
    does the CI loom job. We ran `RUSTFLAGS="--cfg loom" cargo test --lib --release` at 407684b and both models pass.
    The models do catch a lost wake: with `if q.waiting > 0` changed to `if q.waiting > 1` in `send`
    ([src/wake.rs L115 @ e9f73c3](https://github.com/jameshuntt/scope/blob/e9f73c31020cd59736b8b59c47a930f153cf7063/src/wake.rs#L115)), both fail; the one-sender model reports
    `deadlock; threads = [(Id(0), Blocked(Location(None))), (Id(1), Terminated)]`.
    Schedule golden passes at 513cb50. Measured against cfb873a: `mailbox` 14.0% and 16.1% less time (that bench
    queues 100 or 10,000 wakes before `run()`, so draining them is most of the work). Every other bench moved between
    -6.6% and +3.8%, inside the noise.
- **Round overhead: clock only for a pending deadline, one slab lookup per id, smaller task entry.** Every round read
  the clock up to three times with no timer or cancel deadline pending: `fire_expired_cancel_times`,
  `fire_expired_timers` and `next_deadline` each called `Instant::now()` before looking at their heap. The poll loop,
  the wake apply and the timer pass looked a task up twice, once with `get` to check it and once with `get_mut` to
  change it. `TaskEntry` was 144 bytes and a slab slot 152, with `cancel_tick_deadline` an `Option<u64>`.
  `build_report` went through `live_ids()` and looked every id up again, and `live_ids` walked every slot to the end of
  the slab.
  Before: [src/scope.rs L952-953 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L952-L953), [src/scope.rs L974-975 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L974-L975), [src/scope.rs L995-996 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L995-L996) (clock),
  [src/scope.rs L623-633 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L623-L633), [src/scope.rs L667-678 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L667-L678), [src/scope.rs L981-990 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L981-L990) (two lookups),
  [src/scope.rs L73-86 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L73-L86) (entry), [src/ids.rs L32-35 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/ids.rs#L32-L35), [src/ids.rs L120-128 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/ids.rs#L120-L128),
  [src/scope.rs L1008-1014 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L1008-L1014).
  Now the timer and cancel passes return before reading the clock when their heap is empty, and `next_deadline` returns
  `None` before reading it. Those paths, `cancel_now_with_reason`, `request_cancel` and `schedule_cancel_deadlines` look
  each id up once. `TaskEntry` is `repr(C)` with what a round reads for every live task first (`runnable_seq`,
  `last_polled`, `name`, `runnable`, `state`, `qos`), and `cancel_tick_deadline` is `Option<NonZeroU64>`; the deadline
  is the tick count plus a budget of at least one, so it is never zero. `Slot` is `repr(C)` with `generation` first.
  `TaskSlab::entries()` (crate-private) gives `(TaskId, &T)` and stops once it has seen `len` live entries; `live_ids`
  and `build_report` use it.
  After: [src/scope.rs L931-936 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L931-L936), [src/scope.rs L957-962 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L957-L962), [src/scope.rs L978-988 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L978-L988),
  [src/scope.rs L620-623 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L620-L623), [src/scope.rs L657-663 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L657-L663), [src/scope.rs L968-974 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L968-L974),
  [src/scope.rs L74-93 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L74-L93), [src/ids.rs L32-39 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/ids.rs#L32-L39), [src/ids.rs L124-141 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/ids.rs#L124-L141),
  [src/scope.rs L990-991 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L990-L991). Commit [c4fe22d](https://github.com/jameshuntt/scope/commit/c4fe22d20534104ed26b1311789f6cb7b622f21d).
  - Proof: we measured the sizes with a throwaway test (not committed) printing `size_of::<TaskEntry<'static, ()>>()`
    and the slab's `size_of::<Slot<TaskEntry<'static, ()>>>()`: 144 and 152 at da655cf, 136 and 144 at c4fe22d, on
    aarch64-apple-darwin. Schedule golden passes at c4fe22d. Measured against da655cf, every bench moved between
    -4.3% and +3.6%, so on this machine its speed effect is inside the noise.
  - Regression, never published: with many idle tasks c4fe22d made a round slower. In the sparse probe (below) with
    1000 parked tasks, da655cf took 1,425.0 and 1,422.8 µs and c4fe22d took 1,748.1, 1,509.5 and 1,726.3 µs over
    three runs; with 10 and 100 parked tasks c4fe22d was faster. That cost is in the per-round scan of every live
    task, which 967b616 removes, so 0.3.1 does not have it (233.2 µs). We have not pinned down which part of c4fe22d
    causes it.
- **Runnable set: a round costs the runnable tasks, not every live one.** Each round called `order_runnable`, which
  built a `Vec` of every live task (`live_ids`), looked each one up to keep the runnable ones, and sorted with
  comparators that looked both tasks up again on every comparison. The poll loop then looked each id up again. So a
  round cost every live task even when one was runnable.
  Before: [src/scope.rs L613-627 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L613-L627), [src/scope.rs L667-716 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L667-L716).
  Now `RunQueue` holds the runnable set. A task enters when its `runnable` flag goes true (spawn, or a wake applied
  from the bus, which is how timer, finish-waiter and cancel wakes arrive) and leaves when the round takes it. Each
  round reads every member's key once into `(OrderKey, TaskId)` pairs and sorts them with no lookups in the
  comparator. The `TaskId` breaks ties in slot order, which is what the old stable sort over `live_ids` gave.
  `RoundRobin` rotates by the same cursor, and tasks past the poll budget stay queued with `runnable_seq` unchanged.
  A finished task's stale id is dropped at the next reorder.
  After: [src/scope.rs L118-196 @ 967b616](https://github.com/jameshuntt/scope/blob/967b616129a566c1fc8eabf3148b1ed08b2159c6/src/scope.rs#L118-L196), [src/scope.rs L689-716 @ 967b616](https://github.com/jameshuntt/scope/blob/967b616129a566c1fc8eabf3148b1ed08b2159c6/src/scope.rs#L689-L716), [src/scope.rs L741-754 @ 967b616](https://github.com/jameshuntt/scope/blob/967b616129a566c1fc8eabf3148b1ed08b2159c6/src/scope.rs#L741-L754),
  [src/scope.rs L360 @ 967b616](https://github.com/jameshuntt/scope/blob/967b616129a566c1fc8eabf3148b1ed08b2159c6/src/scope.rs#L360). Commit [967b616](https://github.com/jameshuntt/scope/commit/967b616129a566c1fc8eabf3148b1ed08b2159c6).
  - Proof: the old full scan is kept under `cfg(test)` as `full_scan_order` [src/scope.rs L1098-1141 @ 967b616](https://github.com/jameshuntt/scope/blob/967b616129a566c1fc8eabf3148b1ed08b2159c6/src/scope.rs#L1098-L1141), and in
    test builds every round asserts its order equals it [src/scope.rs L691-704 @ 967b616](https://github.com/jameshuntt/scope/blob/967b616129a566c1fc8eabf3148b1ed08b2159c6/src/scope.rs#L691-L704).
    `runnable_set_orders_like_a_full_scan` [src/scope.rs L1234-1284 @ 967b616](https://github.com/jameshuntt/scope/blob/967b616129a566c1fc8eabf3148b1ed08b2159c6/src/scope.rs#L1234-L1284) drives a seeded random workload
    (spawns, wakes of live and finished tasks, timers, finish waiters, soft cancels that escalate by tick, hard cancels
    between rounds) under all 5 orderings and budgets `usize::MAX`, 1 and 3. It passes at 967b616 and at e9f73c3.
    Schedule golden passes at 967b616.
  - Speed, measured against c4fe22d. In the suite the gain is biggest where the old sort did the most lookups:
    `yield_storm` under `BiasedQoSThenAge` (two to six slab lookups per comparison) 8.8%, 25.5% and 31.8% less time
    at 10, 100 and 1000 tasks, under `RoundRobin` 7.4% to 12.4%, `sparse_wake_ring` 16.4% and 17.2% at 10 and 100
    links. The rest moved between -5.6% and +1.4%.
  - The case this commit is for, one or two runnable tasks among many parked ones, is not in the suite (see
    `sparse_wake_ring` under Added). The sparse probe below has two tasks passing 2000 wakes back and forth while 10,
    100 or 1000 other tasks sit parked, then wakes the parked ones so the run finishes: 1002 rounds, every commit the
    same. Median of 101 runs:

    | parked tasks | 0.3.0 (19b484b) | c4fe22d | 967b616 | 0.3.1 (e9f73c3) |
    |---:|---:|---:|---:|---:|
    | 10 | 150.7 µs | 94.5 to 96.9 µs | 73.0 µs | 72.9 µs |
    | 100 | 270.6 µs | 231.3 to 263.8 µs | 88.1 µs | 88.1 µs |
    | 1000 | 1,368.8 µs | 1,509.5 to 1,748.1 µs | 233.7 µs | 233.2 µs |

    0.3.0 to 0.3.1 that is 51.6%, 67.4% and 83.0% less time.

<details>
<summary>The sparse probe (not in the repo; copied into each tree as <code>examples/sparse_probe.rs</code> and run with
<code>cargo run --release --example sparse_probe</code>)</summary>

```rust
// One runnable task among many parked ones: `idle` tasks park on their first poll; two tasks pass
// 2000 wakes back and forth; when the hops run out the last one wakes every parked task.
use scope::{Cx, Scope, ScopedTask, TaskId, TaskPoll, WakeReason};
use std::time::Instant;
struct Env { hops: u64, idle: u32 }
struct Parked { polled: bool }
impl<'env> ScopedTask<'env, Env> for Parked {
    fn poll(&mut self, _cx: &mut Cx<'_, 'env, Env>) -> TaskPoll {
        if self.polled { return TaskPoll::Ready; }
        self.polled = true;
        TaskPoll::Pending
    }
}
struct Ping { other: TaskId }
impl<'env> ScopedTask<'env, Env> for Ping {
    fn poll(&mut self, cx: &mut Cx<'_, 'env, Env>) -> TaskPoll {
        if cx.resources.hops == 0 {
            for i in 0..cx.resources.idle { cx.wake_with(TaskId::new(i, 0), WakeReason::Manual); }
            cx.wake_with(self.other, WakeReason::Manual);
            return TaskPoll::Ready;
        }
        cx.resources.hops -= 1;
        cx.wake_with(self.other, WakeReason::Manual);
        TaskPoll::Pending
    }
}
fn main() {
    for idle in [10u32, 100, 1000] {
        let mut times = Vec::new();
        let mut last = (0, 0, 0);
        for _ in 0..101 {
            let start = Instant::now();
            let mut env = Env { hops: 2000, idle };
            let r = {
                let mut s = Scope::new(&mut env);
                for _ in 0..idle { s.spawn_named("parked", Parked { polled: false }); }
                s.spawn_named("ping", Ping { other: TaskId::new(idle + 1, 0) });
                s.spawn_named("pong", Ping { other: TaskId::new(idle, 0) });
                s.run()
            };
            times.push(start.elapsed().as_nanos());
            last = (r.rounds, r.polls, r.completed);
        }
        times.sort();
        println!("idle={idle} rounds={} polls={} completed={} median_us={:.1}", last.0, last.1, last.2, times[50] as f64 / 1000.0);
    }
}
```

</details>


The README's 0.3.1 section and the version bump are in [e9f73c3](https://github.com/jameshuntt/scope/commit/e9f73c31020cd59736b8b59c47a930f153cf7063).

### Fixed

- **`run()` and `run_until()` lost a wake that arrived while they waited.** When a round polled nothing and a timer or
  cancel deadline was pending, `run()` waited on the bus with `recv_timeout(wait)`. That call takes the message that
  ended the wait off the queue, and `run()` threw it away (`let _ = ...`). A wake from another thread, from a real
  `Waker` or a `MailboxSender::send`, that landed during the wait never reached its task. The task stayed parked, the
  run slept on to the deadline, and the task came back in `RunReport::stalled`. `run_until()` did the same.
  Before: `run()` [src/scope.rs L556-579 @ 513cb50](https://github.com/jameshuntt/scope/blob/513cb506a7cbec53351d27459db014224f2abef8/src/scope.rs#L556-L579), `run_until()` [src/scope.rs L584-606 @ 513cb50](https://github.com/jameshuntt/scope/blob/513cb506a7cbec53351d27459db014224f2abef8/src/scope.rs#L584-L606), and
  `recv_timeout` popping the message [src/wake.rs L170-178 @ 513cb50](https://github.com/jameshuntt/scope/blob/513cb506a7cbec53351d27459db014224f2abef8/src/wake.rs#L170-L178). In 0.3.0: [src/scope.rs L530-580 @ 6fce0fe](https://github.com/jameshuntt/scope/blob/6fce0feab3bac3e107f9cff0763cf92dc5922659/src/scope.rs#L530-L580) and
  [src/wake.rs L123-136 @ 6fce0fe](https://github.com/jameshuntt/scope/blob/6fce0feab3bac3e107f9cff0763cf92dc5922659/src/wake.rs#L123-L136).
  Now the idle wait is `WakeBus::wait_timeout` (crate-private): one lock, returns at once if anything is queued, and
  leaves the message on the bus for the next round's `drain_bus`. With no deadline, `run()` still ends when the bus is
  empty. `recv_timeout` keeps its public behaviour.
  After: [src/scope.rs L556-577 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L556-L577), [src/scope.rs L582-602 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L582-L602), [src/wake.rs L170-175 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/wake.rs#L170-L175).
  Commit [da655cf](https://github.com/jameshuntt/scope/commit/da655cf5434a7a33d427615f6dd89505781b4d9a).
  - Proof: `a_wake_from_another_thread_during_runs_idle_wait_is_not_lost` [tests/kernel.rs L219-258 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/tests/kernel.rs#L219-L258). A task
    parks and hands its real waker to a thread that fires it after 50 ms, while `run()` waits on a 10 s sleep. Once
    woken, the task cancels the sleep. We put this test file into older trees and ran it. On 0.2.2 (d2b31b7), 0.3.0
    (6fce0fe) and 513cb50 it fails after 10.0 s with
    `RunReport { rounds: 5, polls: 3, completed: 1, cancelled: 0, panicked: 0, stalled: [(TaskId { slot: 1, generation: 0 }, "woken")] }`.
    At da655cf it passes in 0.06 s.
  - Where it came from: it was already there in 0.2.0. The two `let _ = self.bus.recv_timeout(wait);` lines,
    [src/scope.rs L545 @ ac32353](https://github.com/jameshuntt/scope/blob/ac323535b42dd274a245266839d7d4ed429b9ad7/src/scope.rs#L545) and [src/scope.rs L579 @ ac32353](https://github.com/jameshuntt/scope/blob/ac323535b42dd274a245266839d7d4ed429b9ad7/src/scope.rs#L579), are unchanged from 0.2.0 through 0.3.0, so every
    published 0.2 and 0.3.0 has it.
- **The `run_scoped` doc test that must not compile now fails for the borrow and nothing else.** The example was a
  `compile_fail` test whose closure had no type on its argument (`move |_cx|`). rustc rejects that closure by itself,
  with "implementation of `FnOnce` is not general enough" and the same for `FnMut`, whatever it borrows. So the test
  passed for the wrong reason, and it would have kept passing with a legal borrow.
  Before: [src/scope.rs L248-263 @ 407684b](https://github.com/jameshuntt/scope/blob/407684bdc7f1bcc360afaf38acf4783f72b3484a/src/scope.rs#L248-L263).
  Now there are two examples that differ only in where the borrowed `String` lives. The first borrows data that
  outlives the scope and has to compile. The second borrows data that dies inside the `run_scoped` closure, is marked
  `compile_fail,E0597`, and types its closure argument (`|_cx: &mut Cx<'_, '_, ()>|`), so the only error left is the
  borrow.
  After: [src/scope.rs L244-283 @ e9f73c3](https://github.com/jameshuntt/scope/blob/e9f73c31020cd59736b8b59c47a930f153cf7063/src/scope.rs#L244-L283). Commit [e9f73c3](https://github.com/jameshuntt/scope/commit/e9f73c31020cd59736b8b59c47a930f153cf7063).
  - Proof: on 0.3.0, `cargo +nightly test --doc run_scoped` shows the old example failing with 5 errors: E0597 plus
    four "not general enough". With `local` moved out of the closure, a legal borrow, it still fails with the four
    "not general enough" errors, so the old `compile_fail` test still passes. On 0.3.1 the second example fails with
    exactly one error, E0597, and the first compiles and runs. Stable rustdoc does not check the `E0597` code on a
    `compile_fail` test, nightly does. The CI test job runs doc tests on stable, so CI enforces the pair (one compiles,
    one does not), not the code.
  - Where it came from: it was already there in 0.2.0, [src/scope.rs L142-157 @ ac32353](https://github.com/jameshuntt/scope/blob/ac323535b42dd274a245266839d7d4ed429b9ad7/src/scope.rs#L142-L157).
- **The loom build of the lib tests failed. Never published.** 967b616 added a `#[cfg(test)] mod tests` to `scope.rs`
  that builds a whole `Scope`. Under `--cfg loom` that module compiled too, and its test touched loom's `Mutex`
  outside a `loom::model`, so `RUSTFLAGS="--cfg loom" cargo test --lib --release`, the line 513cb50 added to
  `verify.sh`, failed with "cannot access Loom execution state from outside a Loom model". 407684b gates the module
  with `#[cfg(all(test, not(loom)))]`.
  Before: [src/scope.rs L1143-1144 @ 967b616](https://github.com/jameshuntt/scope/blob/967b616129a566c1fc8eabf3148b1ed08b2159c6/src/scope.rs#L1143-L1144). After: [src/scope.rs L1143-1145 @ 407684b](https://github.com/jameshuntt/scope/blob/407684bdc7f1bcc360afaf38acf4783f72b3484a/src/scope.rs#L1143-L1145). Commit [407684b](https://github.com/jameshuntt/scope/commit/407684bdc7f1bcc360afaf38acf4783f72b3484a).
  - This was a regression from 967b616, part of 0.3.1 itself. 407684b landed before 0.3.1 was published, so no
    published version has it.
  - Proof: we ran that command at both. At 967b616: 4 passed, 1 failed (`runnable_set_orders_like_a_full_scan`, with
    the message above). At 407684b: 4 passed, 0 failed (the two `ids` tests and both loom models).

## 0.3.0 (2026-10-15 04:43 UTC)

Published from [6fce0fe](https://github.com/jameshuntt/scope/commit/6fce0feab3bac3e107f9cff0763cf92dc5922659); the package's `.cargo_vcs_info.json` names that commit.

### Changed

- **On `waker_tables` 0.3. The waker's vtable is built from `WakeSignal` itself.** 0.2.x built its waker with
  `waker_tables` 0.1.4: a private `vtable` module declared `WAKE_SIGNAL_VTABLE` with `declare_raw_waker_vtable!`, and
  `scheduler_waker` passed it in as `waker_from_arc(arc, &WAKE_SIGNAL_VTABLE)`. In 0.1.4 `waker_from_arc` is a safe fn
  that takes any `&'static RawWakerVTable` (line 162 of `src/lib.rs` in the 0.1.4 package), so nothing tied the vtable
  to the type. In 0.3 it is `waker_from_arc(arc)`, and the vtable comes from `T`. The `vtable` module and
  `WAKE_SIGNAL_VTABLE` are gone.
  Before: [src/wake.rs L161-179 @ bf2b88f](https://github.com/jameshuntt/scope/blob/bf2b88fcaa14194cc2f24dd2e6d793830e2da0e1/src/wake.rs#L161-L179), [Cargo.toml L17 @ bf2b88f](https://github.com/jameshuntt/scope/blob/bf2b88fcaa14194cc2f24dd2e6d793830e2da0e1/Cargo.toml#L17).
  After: [src/wake.rs L162-171 @ 6fce0fe](https://github.com/jameshuntt/scope/blob/6fce0feab3bac3e107f9cff0763cf92dc5922659/src/wake.rs#L162-L171), [Cargo.toml L17 @ 6fce0fe](https://github.com/jameshuntt/scope/blob/6fce0feab3bac3e107f9cff0763cf92dc5922659/Cargo.toml#L17), README note [README.md L155-160 @ 6fce0fe](https://github.com/jameshuntt/scope/blob/6fce0feab3bac3e107f9cff0763cf92dc5922659/README.md#L155-L160).
  Commit [6fce0fe](https://github.com/jameshuntt/scope/commit/6fce0feab3bac3e107f9cff0763cf92dc5922659).
  - This was not a soundness hole in scope. `scheduler_waker` always passed `WakeSignal`'s own vtable, it never takes
    one from a caller, and scope does not re-export `waker_tables`. The version is minor because `WakeSignal` is public
    and implements `waker_tables`' public trait, which changed version (the README says so).
  - Proof: `real_waker_routes_back_to_bus` [src/wake.rs L189-201 @ 6fce0fe](https://github.com/jameshuntt/scope/blob/6fce0feab3bac3e107f9cff0763cf92dc5922659/src/wake.rs#L189-L201) (a clone and the original both wake the
    same task, which goes through the vtable's clone, wake, wake_by_ref and drop) and
    `a_real_external_waker_resumes_a_parked_task` [tests/kernel.rs L199-217 @ 6fce0fe](https://github.com/jameshuntt/scope/blob/6fce0feab3bac3e107f9cff0763cf92dc5922659/tests/kernel.rs#L199-L217). Both pass at 6fce0fe with the
    rest of `cargo test` (lib 4, kernel 13, mailbox 3, doc 3). `cargo +nightly miri test --lib` at 6fce0fe passes too,
    `real_waker_routes_back_to_bus` included. CI runs the lib tests under Miri on every push (run 38692079734 is on
    the 0.3.1 code, whose waker code is the same).

### Fixed

- **`rust-version` is 1.85. It said 1.80, which could not build.** 0.2.0 through 0.2.3 declared
  `rust-version = "1.80"`, but their one dependency, `waker_tables` 0.1.4, is edition 2024, and edition 2024 needs
  Rust 1.85.
  Before: [Cargo.toml L5 @ bf2b88f](https://github.com/jameshuntt/scope/blob/bf2b88fcaa14194cc2f24dd2e6d793830e2da0e1/Cargo.toml#L5). After: [Cargo.toml L5 @ 6fce0fe](https://github.com/jameshuntt/scope/blob/6fce0feab3bac3e107f9cff0763cf92dc5922659/Cargo.toml#L5).
  - Proof: `cargo +1.80 check --locked` on 0.2.2 stops before compiling anything: `failed to parse manifest at
    .../waker_tables-0.1.4/Cargo.toml`, `feature edition2024 is required`. The CI msrv job builds and tests on 1.85
    (run 38692079734).
  - Where it came from: it was already there in 0.2.0, [Cargo.toml L5 @ ac32353](https://github.com/jameshuntt/scope/blob/ac323535b42dd274a245266839d7d4ed429b9ad7/Cargo.toml#L5).

## 0.2.3 (2026-10-15 00:52 UTC)

Published from a copy of the 0.2.2 package, not from a git checkout. The `.crate` has no `.cargo_vcs_info.json`, its
`Cargo.toml.orig` is the 0.2.2 package's normalized manifest with the version changed, and its SHA-256 matches the
crates.io checksum (`f19c3edd...`). Every file besides the two manifests matches [bf2b88f](https://github.com/jameshuntt/scope/commit/bf2b88fcaa14194cc2f24dd2e6d793830e2da0e1) byte for byte.

### Changed

- **The Apache license names its copyright holder.** `LICENSE-APACHE` still had the template line
  `Copyright [yyyy] [name of copyright owner]`. It now reads `Copyright 2026 1o LLC`.
  Before: [LICENSE-APACHE L190 @ d2b31b7](https://github.com/jameshuntt/scope/blob/d2b31b7a4536986df0a1c0fde9fe59ca67ffec7f/LICENSE-APACHE#L190). After: [LICENSE-APACHE L190 @ bf2b88f](https://github.com/jameshuntt/scope/blob/bf2b88fcaa14194cc2f24dd2e6d793830e2da0e1/LICENSE-APACHE#L190). Commit [bf2b88f](https://github.com/jameshuntt/scope/commit/bf2b88fcaa14194cc2f24dd2e6d793830e2da0e1).
  No code changed.

## 0.2.2 (2026-09-03 01:55 UTC)

The last release before the 0.3 work. Published from another repository: the package's `.cargo_vcs_info.json` says
commit `aca125861ea95ad6546f1f428cde4c9c4278f941`, path `rust/published/scope`. [d2b31b7](https://github.com/jameshuntt/scope/commit/d2b31b7a4536986df0a1c0fde9fe59ca67ffec7f) in this repo has the
same files (`src/`, `tests/`, `examples/`, `README.md`, the licenses, `Cargo.lock`, and `Cargo.toml` equal to the
package's `Cargo.toml.orig`).

### Added

- **README: "When to use it".** When scope fits (many small cooperative steps sharing one piece of mutable state, with
  the same schedule every run), and how it differs from an async executor, `std::thread::scope` and rayon's `scope`,
  and a hand-rolled poll loop. [README.md L16-51 @ d2b31b7](https://github.com/jameshuntt/scope/blob/d2b31b7a4536986df0a1c0fde9fe59ca67ffec7f/README.md#L16-L51). No code changed from 0.2.1.

### Known problems, fixed later

- `run()` and `run_until()` drop a wake that ends their idle wait: [src/scope.rs L536-549 @ d2b31b7](https://github.com/jameshuntt/scope/blob/d2b31b7a4536986df0a1c0fde9fe59ca67ffec7f/src/scope.rs#L536-L549),
  [src/scope.rs L565-578 @ d2b31b7](https://github.com/jameshuntt/scope/blob/d2b31b7a4536986df0a1c0fde9fe59ca67ffec7f/src/scope.rs#L565-L578). Fixed in 0.3.1.
- The `run_scoped` `compile_fail` example also fails for a closure error that has nothing to do with the borrow, so
  it would pass with a legal borrow too: [src/scope.rs L140-155 @ d2b31b7](https://github.com/jameshuntt/scope/blob/d2b31b7a4536986df0a1c0fde9fe59ca67ffec7f/src/scope.rs#L140-L155). Fixed in 0.3.1.
- `rust-version = "1.80"` while `waker_tables` 0.1.4 needs 1.85: [Cargo.toml L5 @ d2b31b7](https://github.com/jameshuntt/scope/blob/d2b31b7a4536986df0a1c0fde9fe59ca67ffec7f/Cargo.toml#L5). Fixed in 0.3.0.
