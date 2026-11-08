# Changelog

Every code link below is pinned to a commit, so it opens the lines as they were at that commit. A
"Before" link shows the code with the problem and an "After" link shows the change. Publish times come
from the crates.io API, in UTC. Programs and scripts that are not part of the crate are in
[`proofs/`](proofs/README). Each one has a README with the command and what it prints.

## 0.4.0 (2026-11-08 14:46 UTC)

Published from [34fae6e](https://github.com/jameshuntt/scope/commit/34fae6ec978c53cfc571b9eb56b0cbac5aeb7e1c). The package's `.cargo_vcs_info.json` names that commit. Its commits, oldest
first: d6bf9d2, ae8b8f6, 7100203, b381a2c, 34fae6e; the code is d6bf9d2 and 7100203, and the line links are at 7100203.

Published by the declared route, not by hand: the tag `v0.4.0` at 34fae6e ran the `publish` job, and
crates.io records the version as trusted-published from GitHub, repository `jameshuntt/scope`, run
[42491280364](https://github.com/jameshuntt/scope/actions/runs/42491280364), sha 34fae6e. Every job of
that run passed (test, arm, msrv, miri, loom, latest-deps on stable and on 1.85, publish); the push of
34fae6e to `main` ran [42491112796](https://github.com/jameshuntt/scope/actions/runs/42491112796), which
passed every job, publish skipped. The checksum crates.io records,
`824efa6d588b332f6e4940a3addcf55a083530ab90fd12ab6cad70470815d161`, is the `shasum -a 256` of the
`cargo package --locked` output built on the Mac from the same commit. Breaking: version 0.4.0, because
`WakeReason` gains a variant and is not `#[non_exhaustive]`, so a `match` over it without a `_` arm stops
compiling. Everything else is added.

### Added

- **Capacity waiters on the bounded mailbox.** Commits [d6bf9d2](https://github.com/jameshuntt/scope/commit/d6bf9d218fcdee506ede964af246690cfd1f41d6)
  and [7100203](https://github.com/jameshuntt/scope/commit/710020348ce92865f2811036b0bee4763d9bf1e3) (the fast paths when nobody waits).
  Why: a sender refused `Full` had no way to wait for a place. `try_recv` freed a place and woke
  nobody, so a host kept its own waiter list and woke all of it when a place came free. With 64 senders
  and 16 places, big-bad-bool-runtime 0.1.0 (`run.rs` `release`) spends 22.8 refused attempts per
  commit under `Retry` and 28.8 under `Backpressure`, because every waiter wakes for one place.
  - Before: [src/mailbox.rs L132-135 @ d3c1a6f](https://github.com/jameshuntt/scope/blob/d3c1a6ff68d9126f80aba6e7b2b50ef94ce5bd8f/src/mailbox.rs#L132-L135), `try_recv` pops and wakes nobody;
    [src/mailbox.rs L184-200 @ d3c1a6f](https://github.com/jameshuntt/scope/blob/d3c1a6ff68d9126f80aba6e7b2b50ef94ce5bd8f/src/mailbox.rs#L184-L200), `try_send` refuses `Full` and the sender can only try again;
    [src/mailbox.rs L157-160 @ d3c1a6f](https://github.com/jameshuntt/scope/blob/d3c1a6ff68d9126f80aba6e7b2b50ef94ce5bd8f/src/mailbox.rs#L157-L160) and [src/mailbox.rs L163-173 @ d3c1a6f](https://github.com/jameshuntt/scope/blob/d3c1a6ff68d9126f80aba6e7b2b50ef94ce5bd8f/src/mailbox.rs#L163-L173), `close` and the read end's
    drop set the flag and wake nobody.
  - After: [src/mailbox.rs L127-196 @ 7100203](https://github.com/jameshuntt/scope/blob/710020348ce92865f2811036b0bee4763d9bf1e3/src/mailbox.rs#L127-L196), the waiter table: one entry per task slot, in
    registration order, each registration numbered. [src/mailbox.rs L198-212 @ 7100203](https://github.com/jameshuntt/scope/blob/710020348ce92865f2811036b0bee4763d9bf1e3/src/mailbox.rs#L198-L212): the shared state
    also holds the places held for woken waiters and the bus to wake them through.
  - [src/mailbox.rs L266-288 @ 7100203](https://github.com/jameshuntt/scope/blob/710020348ce92865f2811036b0bee4763d9bf1e3/src/mailbox.rs#L266-L288), `hand_out`: every free place is held for the oldest waiter, one
    each, before any of them is woken (`WakeReason::Capacity`). A waiter registers only while no place
    is free for it, so one freed place wakes one waiter. `try_recv` ([src/mailbox.rs L318-332 @ 7100203](https://github.com/jameshuntt/scope/blob/710020348ce92865f2811036b0bee4763d9bf1e3/src/mailbox.rs#L318-L332))
    and `drain` ([src/mailbox.rs L334-345 @ 7100203](https://github.com/jameshuntt/scope/blob/710020348ce92865f2811036b0bee4763d9bf1e3/src/mailbox.rs#L334-L345)) call it after they take messages; an empty receive
    frees nothing and wakes nothing.
  - [src/mailbox.rs L494-548 @ 7100203](https://github.com/jameshuntt/scope/blob/710020348ce92865f2811036b0bee4763d9bf1e3/src/mailbox.rs#L494-L548), `BoundedSender::wake_when_free(task)`: under the mailbox's lock,
    `Closed`, `FreeNow` when a place is free for the task (nothing registered, so a place freed after
    a refused send is never missed), `AlreadyRegistered` (the task keeps its turn), or `Registered`
    with a `CapacityWait`. A slot's newer generation drops the older generation's entry and held place.
  - [src/mailbox.rs L472-492 @ 7100203](https://github.com/jameshuntt/scope/blob/710020348ce92865f2811036b0bee4763d9bf1e3/src/mailbox.rs#L472-L492), `BoundedSender::try_send_from(msg, task)`: the place held for
    `task` is its to fill, and filling it uses the hold up. [src/mailbox.rs L453-470 @ 7100203](https://github.com/jameshuntt/scope/blob/710020348ce92865f2811036b0bee4763d9bf1e3/src/mailbox.rs#L453-L470): `try_send`
    counts held places as taken, so a sender that does not wait cannot take one; it still uses every
    place nobody holds. [src/mailbox.rs L565-575 @ 7100203](https://github.com/jameshuntt/scope/blob/710020348ce92865f2811036b0bee4763d9bf1e3/src/mailbox.rs#L565-L575): `free` leaves held places out, `free_for(task)`
    adds the task's own, for a caller that checks before it sends.
  - [src/mailbox.rs L402-442 @ 7100203](https://github.com/jameshuntt/scope/blob/710020348ce92865f2811036b0bee4763d9bf1e3/src/mailbox.rs#L402-L442), `CapacityWait`: dropping it (or `cancel`) takes its entry out of
    the table and hands its held place to the next waiter. It is numbered, so a spent registration
    dropped late does not touch a newer one of the same task.
  - [src/mailbox.rs L299-308 @ 7100203](https://github.com/jameshuntt/scope/blob/710020348ce92865f2811036b0bee4763d9bf1e3/src/mailbox.rs#L299-L308) and [src/mailbox.rs L362-382 @ 7100203](https://github.com/jameshuntt/scope/blob/710020348ce92865f2811036b0bee4763d9bf1e3/src/mailbox.rs#L362-L382): `close` and the read end's drop
    let every held place go and wake every waiter, oldest first; a registration on a closed mailbox is
    `Closed`. [src/wake.rs L55-57 @ 7100203](https://github.com/jameshuntt/scope/blob/710020348ce92865f2811036b0bee4763d9bf1e3/src/wake.rs#L55-L57): `WakeReason::Capacity`, last, so the other variants keep their
    discriminants (the pinned schedule test hashes them).
  - A held place and a task that ends without sending: the experiment found that a waiter woken for a
    place that it never used, and that never gave it up, kept that place from everyone. Here the place
    is given up when the `CapacityWait` is dropped. Kept in the waiting task's own state, it is dropped
    when Scope drops the task, which happens on every way a task ends (it completes, is cancelled or
    panics), so the place goes to the next waiter. The cost: the caller keeps one value per registration
    (an `Arc` clone of the mailbox's shared state, no allocation) and its drop takes the mailbox's lock
    once. A `CapacityWait` kept outside the task holds its place until it is dropped; the scheduler is
    not involved.
  - Unchanged when nobody waits: no waiter registered means no held place, so `try_send`, `try_recv`,
    `drain`, `free` and `close` do what they did in 0.3.2, with the same locks; a refused send still
    allocates nothing (`a_refused_send_allocates_nothing_and_wakes_nothing` runs as written). At
    d6bf9d2 `try_recv` built its wake list even with nobody waiting: big-bad-bool-runtime's
    `scheduler/scope_bounded_bounce` bench (10,000 messages through capacity 4, no waiter) took 516 and
    518 µs against 492 and 489 µs on 0.3.2; 7100203 returns before that, 491 and 495 µs. The
    scheduler, `WakeBus` and `waker_tables` are not changed; the dependency tree is the same.
- Where the numbers come from. The design was measured first as an experiment (attempts_scope_wake_n,
  run wake-n-20261107T185418Z, 610 nodes): FIFO wake-N with the place held for the woken waiter
  ("fifo-reserve") passed every hard gate in all 32 main-grid trials, with no waiter waiting past the
  starvation bound, against 696 such waits on the wake-all control; plain FIFO without the held place
  passed 18 of 32 (a sender that did not wait took the place freed for a woken one). Across the main
  grid fifo-reserve woke 41x, refused 27x and polled 18x less per commit than the control. Then the real
  runtime was run on it (attempts_runtime_wake_validation, run runtime-wake-20261108T131623Z, 56 nodes,
  Custodian 0.8.0): big-bad-bool-runtime 0.1.0 changed only where its waiter list became these calls.
  On its `policy/fan_in_64x16` shape refused attempts per commit went from 22.82 to 2.81 (`Retry`) and
  from 28.83 to 0.984 (`Backpressure`), wakes per commit from 21.85 and 28.83 to 0.984, longest wait
  from 994 to 65 rounds; at 1000 senders and one place, 500 to 0.999. Uncontended cost per commit
  moved by -1.6% to +1.6%. Every gate held: no message lost or taken twice, the bound never exceeded,
  no wake lost, no stale waiter reaching its slot's next task, repeated runs equal.
- Proof: the tests in [src/mailbox.rs L754-1170 @ 7100203](https://github.com/jameshuntt/scope/blob/710020348ce92865f2811036b0bee4763d9bf1e3/src/mailbox.rs#L754-L1170) (16, among them one place wakes one, N wake N,
  registration order, a held place cannot be stolen, a second registration, cancel before and after the
  wake, the handoff, close, a stale generation, four threads sending and waiting, a drain, zero
  capacity, and sends without waiters as before), the loom models in
  [src/mailbox.rs L1172-1389 @ 7100203](https://github.com/jameshuntt/scope/blob/710020348ce92865f2811036b0bee4763d9bf1e3/src/mailbox.rs#L1172-L1389) (4: a registration racing a receive and a sender that does not
  wait, a waiter giving up while a place is freed, a close racing a registration, two waiters racing a
  receive), and `tests/capacity_waiters.rs` (3, on a running scope under every ordering: every message
  once in each sender's order and every wake finding its place, the same run twice, and tasks that end
  holding a place handing it on). `./verify.sh` passes: 63 tests and 6 doc tests (the README's example of a waiting sender among them), Miri on the lib (27,
  the four-thread test skipped) and the kernel tests (13), loom 2 + 10. Miri also passes
  `tests/capacity_waiters.rs`; the tests pass on Rust 1.85.

## 0.3.2 (2026-11-07 13:53 UTC)

Published from [8e3449b](https://github.com/jameshuntt/scope/commit/8e3449ba4c78185a3315d45d15ca1be75e0ed96a). The package's `.cargo_vcs_info.json` names that commit. Its commits, oldest
first: 782814c, 8a6aa20, 3096224, 52ea93d, 1468b17, 320c5bd, c5b1bbd, d5dc5a1, b3ba4b8, 8e3449b.

Published by the declared route, not by hand: the tag `v0.3.2` at 8e3449b ran the `publish` job, and
crates.io records the version as trusted-published from GitHub, repository `jameshuntt/scope`, run
[42378892451](https://github.com/jameshuntt/scope/actions/runs/42378892451), sha 8e3449b. Every job of
that run passed (test, arm, msrv, miri, loom, latest-deps on stable and on 1.85, publish); the push of
8e3449b to `main` ran [42378770724](https://github.com/jameshuntt/scope/actions/runs/42378770724), which
passed every job, publish skipped. The checksum crates.io records,
`bc4e9649ae7f5a91c5ef255ba336f169ffe05c84cd145e901a6ddd5c27a21a2d`, is the `shasum -a 256` of the
`cargo package --locked` output built on the Mac from the same commit. Additive: version 0.3.2.

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

- **A logical clock.** Commit
  [c5b1bbd](https://github.com/jameshuntt/scope/commit/c5b1bbd2d4bf73eaeff10864687fecee37e2e340).
  Why: every timer read the wall clock, so where a timer wake landed among other work, and how many
  rounds a run spent waiting, depended on elapsed time. A run with timers did not repeat.
  - Before: [src/timer.rs L20-33 @ 320c5bd](https://github.com/jameshuntt/scope/blob/320c5bd961598edba4f5eb6a628d40a1c62f5c71/src/timer.rs#L20-L33), `SleepTask` reads
    `Instant::now()`; [src/scope.rs L1015-1034 @ 320c5bd](https://github.com/jameshuntt/scope/blob/320c5bd961598edba4f5eb6a628d40a1c62f5c71/src/scope.rs#L1015-L1034),
    `fire_expired_timers` compares the sleep heap with `Instant::now()`.
  - The scope already counted rounds: [src/scope.rs L697-701 @ 320c5bd](https://github.com/jameshuntt/scope/blob/320c5bd961598edba4f5eb6a628d40a1c62f5c71/src/scope.rs#L697-L701)
    (`tick_count` goes up by one per round) and [src/scope.rs L968-987 @ 320c5bd](https://github.com/jameshuntt/scope/blob/320c5bd961598edba4f5eb6a628d40a1c62f5c71/src/scope.rs#L968-L987)
    (tick-based cancel escalation fires from that count).
  - After: [src/clock.rs @ c5b1bbd](https://github.com/jameshuntt/scope/blob/c5b1bbd2d4bf73eaeff10864687fecee37e2e340/src/clock.rs): `Tick` and `TickDelta`. A tick is the number of a
    scheduler round. The one rule: the clock advances by one at the start of every round (each `tick()`,
    each loop of `run` and `run_until`, idle rounds included), and nothing else moves it. Tick 0 before
    the first round.
  - [src/task.rs L186-208 @ c5b1bbd](https://github.com/jameshuntt/scope/blob/c5b1bbd2d4bf73eaeff10864687fecee37e2e340/src/task.rs#L186-L208): `Cx::now_tick`, `Cx::sleep_ticks`
    (`delta` rounds after this one; 0 counts as 1) and `Cx::sleep_until_tick` (a tick not after this round
    counts as the next round). [src/timer.rs L36-80 @ c5b1bbd](https://github.com/jameshuntt/scope/blob/c5b1bbd2d4bf73eaeff10864687fecee37e2e340/src/timer.rs#L36-L80): `TickSleepTask`,
    beside `SleepTask`. [src/scope.rs L353-357, L458-468 @ c5b1bbd](https://github.com/jameshuntt/scope/blob/c5b1bbd2d4bf73eaeff10864687fecee37e2e340/src/scope.rs#L353-L468):
    `Scope::now_tick`, `Scope::sleep_ticks`, `Scope::sleep_until_tick`.
  - [src/scope.rs L1087-1125 @ c5b1bbd](https://github.com/jameshuntt/scope/blob/c5b1bbd2d4bf73eaeff10864687fecee37e2e340/src/scope.rs#L1087-L1125): logical timers live in a heap
    ordered by `(tick, TaskId)`, like the cancel tick heap, and fire at the start of their round, right
    after the cancel ticks and before the bus is drained, as `WakeReason::Timer` wakes on the bus. The
    policy orders them with the round's other wakes. A replaced timer, or one of a finished task, is dropped.
  - [src/scope.rs L733-739 @ c5b1bbd](https://github.com/jameshuntt/scope/blob/c5b1bbd2d4bf73eaeff10864687fecee37e2e340/src/scope.rs#L733-L739): when nothing ran and a live logical
    timer waits, `run` runs the next round at once instead of waiting on the clock or ending.
  - [src/trace.rs L45-50 @ c5b1bbd](https://github.com/jameshuntt/scope/blob/c5b1bbd2d4bf73eaeff10864687fecee37e2e340/src/trace.rs#L45-L50): `Tracer::on_sleep_ticks` (armed, after
    the poll) and `Tracer::on_tick_timer` (fired, before its wake goes on the bus). `BufferTracer` records
    both. `on_sleep_until` and `on_subscribe_finish` are still not called: calling them would change what
    the pinned schedule's tracer records.
  - Not used: chrono_macros 0.1.0. Its types wrap `chrono::DateTime<Utc>` (the calendar clock, which can
    jump) and `chrono::Duration` (signed). The real-time timers here use `std::time::Instant` (monotonic)
    and `std::time::Duration`, which already say what they are; the change would add chrono to a crate whose
    only dependency is waker_tables and gain no type safety. Ticks never use it.
- **A bounded mailbox.** Same commit. Why: `MailboxSender::send` always pushes and the sender cannot
  see the backlog, so a host had to count it itself.
  - Before: [src/mailbox.rs L56-61 @ 320c5bd](https://github.com/jameshuntt/scope/blob/320c5bd961598edba4f5eb6a628d40a1c62f5c71/src/mailbox.rs#L56-L61).
  - After: [src/mailbox.rs L100-262 @ c5b1bbd](https://github.com/jameshuntt/scope/blob/c5b1bbd2d4bf73eaeff10864687fecee37e2e340/src/mailbox.rs#L100-L262): `BoundedMailbox`,
    `BoundedSender`, `TrySendError`. The capacity is fixed at spawn and the queue is allocated once at it.
    `try_send` enqueues and sends the same `Inbox` wake as `send`, or gives the message back as `Full` or
    `Closed` and enqueues, wakes and allocates nothing. Receiving frees a place at once. Dropping the read
    end (the task completing, cancelled or panicking) closes the mailbox and drops what is queued; `close`
    closes it from inside. Capacity 0 refuses every message. `len`, `capacity`, `free` and `is_closed` on
    the sender. [src/scope.rs L513-557 @ c5b1bbd](https://github.com/jameshuntt/scope/blob/c5b1bbd2d4bf73eaeff10864687fecee37e2e340/src/scope.rs#L513-L557):
    `Scope::spawn_with_bounded_mailbox` and `spawn_with_bounded_mailbox_result`. `Mailbox` and
    `MailboxSender` are unchanged.
  - Proof: `tests/schedule_golden.rs` passes unchanged, so no existing schedule moved.
    [tests/logical_clock.rs @ c5b1bbd](https://github.com/jameshuntt/scope/blob/c5b1bbd2d4bf73eaeff10864687fecee37e2e340/tests/logical_clock.rs) (8 tests: a tick is the round number,
    a timer fires in its round through the bus, 0 and past ticks, a later request replaces the earlier, a
    cancelled sleeper does not keep `run` going, `TickSleepTask` joins, ten runs equal tick for tick under
    all 5 policies and 3 budgets, timer wakes ordered by the policy).
    [tests/bounded_mailbox.rs @ c5b1bbd](https://github.com/jameshuntt/scope/blob/c5b1bbd2d4bf73eaeff10864687fecee37e2e340/tests/bounded_mailbox.rs) (6 tests: capacity 0, exactly
    capacity and one more, send after receive, closed recipients, the same trace as the unbounded mailbox
    when nothing is refused, ten equal runs with refusals under every policy and budget). Unit tests in
    `src/mailbox.rs`: a refused send allocates and wakes nothing over 150 sends, dropping the read end
    closes it. Loom models in `src/mailbox.rs`: a delivered `Inbox` wake means the message is queued; the
    bound holds and every accepted message is received once.
  - Run on the Mac: `cargo test --all-targets --locked` 41 passed; `cargo test --doc --locked` 5 passed (with the README example added in the next commit);
    clippy `-D warnings`; rustdoc `-D warnings`; `cargo bench --no-run`; `cargo package --locked`;
    `cargo +1.85 test --all-targets --locked` 41 passed; loom `--test loom_wake` 2 passed, `--lib` 6
    passed, clippy under loom; Miri `--lib` 9 passed, `--test kernel` 13 passed.
- **The logical clock stops at `u64::MAX` instead of wrapping.** Commit
  [b3ba4b8](https://github.com/jameshuntt/scope/commit/b3ba4b89605c75f8b58b42a89fe90ca676d01855).
  Why: the round counter went up with `wrapping_add`, so after `u64::MAX` rounds the clock would read
  tick 0, and every pending logical timer and tick-based cancel deadline would be out of reach until it
  came round again.
  - Before: [src/scope.rs L698 @ 320c5bd](https://github.com/jameshuntt/scope/blob/320c5bd961598edba4f5eb6a628d40a1c62f5c71/src/scope.rs#L698), `self.tick_count = self.tick_count.wrapping_add(1)`.
  - After: [src/scope.rs L790-791 @ b3ba4b8](https://github.com/jameshuntt/scope/blob/b3ba4b89605c75f8b58b42a89fe90ca676d01855/src/scope.rs#L790-L791), `saturating_add(1)`.
    [src/clock.rs L9-23 @ b3ba4b8](https://github.com/jameshuntt/scope/blob/b3ba4b89605c75f8b58b42a89fe90ca676d01855/src/clock.rs#L9-L23): the clock stops at
    `u64::MAX`; every round after the one at `u64::MAX` is tick `u64::MAX` again. A deadline past it
    (`now + delta` overflowing, which `Cx::sleep_ticks`, `TickSleepTask` and `Tick + TickDelta` already
    saturated) is `u64::MAX`, so the timer fires in the first round at `u64::MAX`, or in the next round
    when it was asked for there. Saturating, not a terminal error: `round` has no error path, and
    saturating is what the tick cancel deadlines and `Tick + TickDelta` already do. Every timer still
    fires and every outcome stays deterministic.
  - Proof: [src/scope.rs L1461-1543 @ b3ba4b8](https://github.com/jameshuntt/scope/blob/b3ba4b89605c75f8b58b42a89fe90ca676d01855/src/scope.rs#L1461-L1543), three tests
    through a test-only `Scope::set_tick`: `the_clock_stops_at_the_last_tick_and_a_timer_past_it_fires_there`
    (from `u64::MAX - 2`, a 10-tick timer fires in round `u64::MAX`; 2 rounds),
    `at_the_last_tick_every_round_is_that_tick_and_timers_still_fire` (at `u64::MAX`, `tick()` leaves the
    clock at `u64::MAX` and a 3-tick timer fires the next round), and
    `a_tick_sleep_task_past_the_end_completes_at_the_last_tick` (`sleep_ticks(TickDelta(u64::MAX))` and
    `sleep_until_tick(Tick(u64::MAX))` both complete in round `u64::MAX`; 3 rounds).
- **Determinism, said where it is read.** Same commit. [README.md L189-194 @ b3ba4b8](https://github.com/jameshuntt/scope/blob/b3ba4b89605c75f8b58b42a89fe90ca676d01855/README.md#L189-L194)
  and [src/lib.rs L20-23 @ b3ba4b8](https://github.com/jameshuntt/scope/blob/b3ba4b89605c75f8b58b42a89fe90ca676d01855/src/lib.rs#L20-L23): with logical timers,
  the tick at which each thing happens is a function of the wake sequence, the policy and the
  tick-advancement rule; wall-clock timers and cancel escalation by time are outside that replay.
- **Formatting.** Same commit. The lines c5b1bbd added are rustfmt-clean (35 rustfmt hunks, all inside
  lines this branch added or changed). No older line is reformatted: `cargo fmt --check` flags the same
  older lines it flags on 320c5bd, less the two lines c5b1bbd itself rewrote.
- **Checked before release** (the code, not the description):
  - The clock reads tick 0 before the first round and goes up by one at one place, the first line of
    `round` (each `tick()`, each loop of `run` and `run_until`). Nothing on the tick path reads `Instant`,
    `SystemTime` or `Duration`: the lines c5b1bbd added name them only in docs.
  - Logical timers live in a heap of `(tick, TaskId)`; two timers due in the same round fire in `TaskId`
    order (slot, then generation), right after the tick cancels and before the bus is drained, as
    `WakeReason::Timer` wakes on the bus, so the policy orders them with the round's other wakes
    (`tests/logical_clock.rs` `timer_wakes_are_ordered_by_the_policy_like_any_wake`).
  - `SleepTask`, the wall-clock sleep heap and `fire_expired_timers` are unchanged; so is
    `tests/schedule_golden.rs`, and it passes.
  - `Cx::sleep_ticks(TickDelta(0))` counts as 1 (documented; tested by
    `zero_ticks_is_the_next_round_and_a_past_tick_is_the_next_round`); `TickSleepTask::new(TickDelta(0))`
    completes at its first poll (documented; tested by `tick_sleep_tasks_join_like_sleep_tasks`).
  - The two new `Tracer` hooks have default bodies, so no implementation has to change. `Cx`'s new
    fields are crate-private, as its other machinery is. `SchedulerConfig` and every other public struct
    gained no field; nothing is `#[non_exhaustive]`. Every change is an addition: version 0.3.2.
  - Bounded mailbox: the bound is `msgs.len() >= capacity`, checked under the lock before
    `push_back`, and `try_send` is the only path that enqueues; the `VecDeque` is made with
    `with_capacity(capacity)` and never grows (a unit test checks its allocated capacity over 150
    sends). `Closed` is checked before `Full`. A refused message is returned by value, never cloned or
    queued, and wakes nothing. Capacity 0 refuses every message (documented; `capacity_zero_refuses_every_send`).
    The wake is the unbounded mailbox's `Inbox` wake, after the lock is released, as `send` does
    (`wake_ordering_is_the_unbounded_mailbox_s_when_nothing_is_refused` compares the two traces).
    `Mailbox` and `MailboxSender` are unchanged.
  - `on_sleep_until` and `on_subscribe_finish` are still never called, as before.
- **The bounded mailbox beside ABUT's ring** (abut 0.2.2, `ring` feature `ring-heapless`, read from its
  published source). Both refuse instead of growing, and both report a full queue as a value the
  sender handles (`TrySendError::Full(msg)` here, `RingError::Full` there). They differ where scope's
  job differs. ABUT's `HeaplessRing<FRAME_MAX, DEPTH>` never allocates at all: capacity is a const
  generic, frames are stored in place (`heapless::spsc::Queue`), and it is single producer, single
  consumer, lock-free. Scope's bounded mailbox allocates once, at spawn, for a capacity chosen at run
  time, then never again; it is many producers (cloneable `BoundedSender`, any thread) behind the
  scope's `Mutex`, and it hands the refused message back. A future `no_std` scope could reuse ABUT's
  pattern for the queue's storage (a const-generic inline ring, so the capacity is in the type and the
  spawn allocates nothing), but not as it is: scope's senders are many and cross threads, so it would
  need a multi-producer ring or a critical section around one, and the rest of scope (`Arc`, `Mutex`,
  `Instant`, the waker substrate) needs `alloc` and a lock first. Nothing changes now.
  - Run on the Mac at b3ba4b8: `./verify.sh` passed (`cargo test` 49 passed and 1 doc test ignored;
    Miri `--lib` 12 passed, `--test kernel` 13 passed with `sleep_task` skipped; loom `--test loom_wake` 2
    passed, `--lib` 6 passed). Miri on the new tests, with the same `MIRIFLAGS`: `--test logical_clock` 8
    passed, `--test bounded_mailbox` 6 passed. `cargo test --all-targets --locked` 44 passed;
    `cargo test --doc --locked` 5 passed, 1 ignored; `cargo +1.85 test --all-targets --locked` 44 passed;
    clippy `-D warnings` on all targets and on the loom lib build; rustdoc `-D warnings`;
    `cargo bench --no-run`.

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
- The `run_scoped` doc examples now pin the bound from both sides. A borrow of data that outlives the
  scope compiles. The same task borrowing data that dies first fails with E0597 and nothing else. See
  Fixed below.

Neither 0.2.2 nor 0.3.1 writes out an `unsafe` block, fn or impl in `src/`. Outside comments,
`git grep -n unsafe` on `src/` at d2b31b7 and at e9f73c3 finds only `#![deny(unsafe_code)]` in `lib.rs`,
`#![allow(unsafe_code)]` in `wake.rs`, and the import and call of `derive_arc_unsafe_wakeable!` in
`wake.rs`. The `unsafe` code is what `waker_tables`' macros generate in `wake.rs`. The
`derive_arc_unsafe_wakeable!(WakeSignal, emit)` line reads the same in both, though 0.3.0 moved to
`waker_tables` 0.3. The `declare_raw_waker_vtable!` call went away in 0.3.0 (see that version below).

One changed line has the word `unsafe` in it, a doc comment in `src/scope.rs`.
`git diff d2b31b7 e9f73c3 -- src | grep '^[-+].*unsafe'` prints its old and new wording:

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
    [`proofs/sparse-probe`](proofs/sparse-probe/README), under the runnable set entry below.

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
  name (`e.name.clone()`, a `String` allocation) and its cancel cell's `Arc`. It built `Cx` with a clone
  of the bus `Arc` and a second clone of the cell `Arc`. All three `Arc` clones were dropped again when
  the poll ended. It also made four new `Vec`s for the intents, so a poll that spawned, slept,
  subscribed or cancelled allocated them fresh.
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
  - Proof: `schedule_golden` passes at cfb873a. Measured against 19b484b: `yield_storm` 13.5% to 21.4%
    less time, `sparse_wake_ring` 19.3% to 22.2%, `spawn_churn` 19.3% to 23.5%, `join_all` 12.6% to 14.7%.
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
  again after each batch, until a take comes back empty. A tracer hook is the only code on the scheduler
  thread that can send while a batch is applied, so this keeps those wakes in the same round, as taking
  them one at a time did.
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
  - `schedule_golden` passes at 513cb50. Measured against cfb873a: `mailbox` 14.0% and 16.1% less time.
    That bench sends 100 or 10,000 messages before `run()`, and each send queues a wake on the bus. Every
    other bench moved between -6.6% and +3.8%, and we do not claim those.
- **Round overhead: clock only for a pending deadline, one slab lookup per id, smaller task entry.**
  Every round read the clock up to three times with no timer or cancel deadline pending.
  `fire_expired_cancel_times`, `fire_expired_timers` and `next_deadline` each called `Instant::now()`
  before looking at their heap. The poll loop in `round` and `fire_expired_timers` looked a task up
  twice, with `get` to check it and `get_mut` to change it. `apply_wake_batch` called `get_mut` twice.
  `TaskEntry` was 144 bytes and a slab slot 152, with `cancel_tick_deadline` an `Option<u64>`.
  `build_report` went through `live_ids()` and looked every id up again. `live_ids` walked every slot
  to the end of the slab.
  - Before: [src/scope.rs L952-953 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L952-L953), `fire_expired_cancel_times` reading the clock first.
  - Before: [src/scope.rs L974-975 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L974-L975), `fire_expired_timers` reading the clock first.
  - Before: [src/scope.rs L995-996 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L995-L996), `next_deadline` reading the clock first.
  - Before: [src/scope.rs L623-633 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L623-L633), the poll loop in `round`: `get`, then `get_mut`.
  - Before: [src/scope.rs L667-678 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L667-L678), `apply_wake_batch`: `get_mut` twice.
  - Before: [src/scope.rs L981-990 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L981-L990), `fire_expired_timers`: `get`, then `get_mut`.
  - Before: [src/scope.rs L73-86 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L73-L86), `TaskEntry` with `cancel_tick_deadline: Option<u64>`.
  - Before: [src/ids.rs L32-35 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/ids.rs#L32-L35), `Slot`.
  - Before: [src/ids.rs L120-128 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/ids.rs#L120-L128), `live_ids` walking every slot.
  - Before: [src/scope.rs L1008-1014 @ da655cf](https://github.com/jameshuntt/scope/blob/da655cf5434a7a33d427615f6dd89505781b4d9a/src/scope.rs#L1008-L1014), `build_report` looking each id up again.

  Now `fire_expired_cancel_times` and `fire_expired_timers` return before reading the clock when their
  heap is empty. `next_deadline` returns `None` before reading it. The poll loop in `round`,
  `apply_wake_batch` and `fire_expired_timers` look each id up once, and so do
  `cancel_now_with_reason`, `request_cancel` and `schedule_cancel_deadlines`. `TaskEntry` is
  `repr(C)`, with what a round reads for every live task first: `runnable_seq`, `last_polled`, `name`,
  `runnable`, `state`, `qos`. `cancel_tick_deadline` is `Option<NonZeroU64>`. The deadline is the tick
  count plus a budget of at least one, so it is never zero. `Slot` is `repr(C)` with `generation` first.
  `TaskSlab::entries()` (crate-private) gives `(TaskId, &T)` and stops once it has seen `len` live
  entries. `live_ids` and `build_report` use it.
  - After: [src/scope.rs L931-936 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L931-L936), `fire_expired_cancel_times` returning early on an empty
    heap.
  - After: [src/scope.rs L957-962 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L957-L962), `fire_expired_timers` returning early on an empty heap.
  - After: [src/scope.rs L978-988 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L978-L988), `next_deadline` returning `None` before the clock.
  - After: [src/scope.rs L620-623 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L620-L623), the poll loop in `round`: one `get_mut`.
  - After: [src/scope.rs L657-663 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L657-L663), `apply_wake_batch`: one `get_mut`.
  - After: [src/scope.rs L968-974 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L968-L974), `fire_expired_timers`: one `get_mut`.
  - After: [src/scope.rs L74-93 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L74-L93), `TaskEntry` as `repr(C)` with the per-round fields first.
  - After: [src/ids.rs L32-39 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/ids.rs#L32-L39), `Slot` as `repr(C)` with `generation` first.
  - After: [src/ids.rs L124-141 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/ids.rs#L124-L141), `live_ids` built on `entries()`, which stops after `len` live
    entries.
  - After: [src/scope.rs L990-991 @ c4fe22d](https://github.com/jameshuntt/scope/blob/c4fe22d20534104ed26b1311789f6cb7b622f21d/src/scope.rs#L990-L991), `build_report` through `entries()`.
  - Commit [c4fe22d](https://github.com/jameshuntt/scope/commit/c4fe22d20534104ed26b1311789f6cb7b622f21d).
  - Proof: [`proofs/entry-size`](proofs/entry-size/README) prints
    `TaskEntry 144 bytes, slab slot 152 bytes` at da655cf and `TaskEntry 136 bytes, slab slot 144 bytes`
    at c4fe22d, on aarch64-apple-darwin. `schedule_golden` passes at c4fe22d. Measured against da655cf,
    every bench moved between -4.3% and +3.6%, and we do not claim a speed change from it.
  - Regression, never published: with 1000 parked tasks, c4fe22d is slower than da655cf in most runs of
    [`proofs/sparse-probe`](proofs/sparse-probe/README) (below), and by how much moves a lot from run to
    run. In the first three runs each, da655cf took 1,419.5 to 1,440.0 µs
    and c4fe22d took 1,965.6 to 2,003.8 µs. In eight more runs each, the two taken in turn, da655cf took
    1,443.2 to 1,461.0 µs and c4fe22d took 1,555.2 to 1,929.4 µs, 7.2% to 32.6% more run for run. A re-run
    on 2026-10-16, 40 runs each, had c4fe22d slower in 38 and faster in 2 (by 1.5% and 2.4%), +17.4% at
    the median. Its times come in two levels, and in about a third of its runs it is within a few percent
    of da655cf. So it is slower at 1000 as a rule, not every time, and we cannot give one figure for how
    much.
  - With 100 parked tasks we claim no difference. In the eight runs c4fe22d took 236.3 to 256.2 µs
    against 242.7 to 256.4 µs, and it was faster in 3 of them. With 10 parked tasks c4fe22d was faster
    in every run: 93.0 to 97.1 µs against 108.6 to 117.3 µs. All eleven runs are in the probe's README.
  - 967b616 removes the per-round scan of every live task, and 0.3.1 does not show the slowdown (231.6
    to 236.4 µs at 1000). We have not found which part of c4fe22d causes it. Not in 0.2.2; it came in
    with c4fe22d.
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
  - After: [src/scope.rs L689-716 @ 967b616](https://github.com/jameshuntt/scope/blob/967b616129a566c1fc8eabf3148b1ed08b2159c6/src/scope.rs#L689-L716), `round`: `reorder`, then `take_next` up to the budget.
  - After: [src/scope.rs L741-754 @ 967b616](https://github.com/jameshuntt/scope/blob/967b616129a566c1fc8eabf3148b1ed08b2159c6/src/scope.rs#L741-L754), `apply_wake_batch` putting a woken task in `RunQueue`.
  - After: [src/scope.rs L360 @ 967b616](https://github.com/jameshuntt/scope/blob/967b616129a566c1fc8eabf3148b1ed08b2159c6/src/scope.rs#L360), `insert_task` putting a spawned task in `RunQueue`.
  - Commit [967b616](https://github.com/jameshuntt/scope/commit/967b616129a566c1fc8eabf3148b1ed08b2159c6).
  - Proof: the old full scan is kept under `cfg(test)` as `full_scan_order`, and in test builds every
    round asserts that its order equals it. `runnable_set_orders_like_a_full_scan` drives a seeded random
    workload under all 5 orderings and budgets `usize::MAX`, 1 and 3: spawns, wakes of live and finished
    tasks, timers, finish waiters, soft cancels that escalate by tick, and hard cancels between rounds.
    `cargo test --lib` passes it at 967b616 and at e9f73c3. `schedule_golden` passes at 967b616.
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

    The c4fe22d cells come from its first three runs. In eight later runs it took 1,555.2 to 1,929.4 µs
    at 1000, and at 100 it overlapped da655cf. See the regression note under the round overhead entry.

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
  for `FnMut`, whatever it borrows and with no borrow at all. So the test passed for the wrong reason,
  and it would have kept passing with a legal borrow.
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
