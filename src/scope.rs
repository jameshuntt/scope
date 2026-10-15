//! The scheduler: a single-threaded, cooperative, deterministic executor whose
//! tasks borrow the scope environment `R`.
//!
//! How it is put together:
//! * Tasks live in a generational [`TaskSlab`]: a wake or join that outlives
//!   its task resolves to `None`, so no deadline needs a validity check.
//! * The runnable set is ordered every round by the [`WakeOrdering`] policy;
//!   this is the axis the scheduler policy chooses on.
//! * `run()` drives to completion, so the scope boundary is a real join point
//!   (structured concurrency), and reports any tasks that could not finish
//!   instead of hanging.
//! * Waking rides the [`WakeBus`] / real-`Waker` substrate.
//! * Zero `unsafe`: the mid-poll callbacks record *intents* into buffers
//!   applied after the poll, so no closure needs `&mut` into the scheduler while
//!   a task holds the environment borrow. The scope keeps those buffers and
//!   reuses them from poll to poll.

use crate::cancel::{CancelCell, CancelEscalation, CancelReason};
use crate::combinators::{
    result_channel, Either, JoinAllOkError, JoinAllOkTask, JoinAllTask, JoinHandle, MapJoinTask,
    Select2Task, SelectNTask,
};
use crate::ids::{TaskId, TaskSlab};
use crate::mailbox::{Mailbox, MailboxSender};
use crate::policy::{resolve_cancel_budgets, QoSClass, SchedulerConfig, WakeOrdering};
use crate::sync::{lock, Arc, Mutex};
use crate::task::{Cx, JoinError, ScopedTask, TaskPoll, TaskState};
use crate::timer::SleepTask;
use crate::trace::Tracer;
use crate::wake::{SchedulerMsg, WakeBus, WakeReason};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};
use std::num::NonZeroU64;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::{Duration, Instant};

/// Summary of a completed `run()`. `stalled` is non-empty only if tasks were
/// still waiting on a wake that never came (a bug in a task, not a hang in the
/// runtime).
#[derive(Debug, Clone, Default)]
pub struct RunReport {
    /// Scheduler rounds run.
    pub rounds: u64,
    /// Task polls performed.
    pub polls: u64,
    /// Tasks that finished with `Ready`.
    pub completed: usize,
    /// Tasks hard-stopped by cancellation.
    pub cancelled: usize,
    /// Tasks whose poll panicked; the panic was isolated.
    pub panicked: usize,
    /// Tasks still pending when no wake could ever arrive, with their names.
    pub stalled: Vec<(TaskId, String)>,
}

impl RunReport {
    /// Whether every task completed: nothing cancelled, panicked or stalled.
    pub fn all_completed(&self) -> bool {
        self.cancelled == 0 && self.panicked == 0 && self.stalled.is_empty()
    }
}

/// Outcome of a single manual round via [`Scope::tick`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickOutcome {
    /// Polled at least one task.
    Progress(usize),
    /// Nothing was runnable this round, but tasks remain.
    Idle,
    /// No tasks remain.
    Done,
}

/// `repr(C)` keeps this order. First what every round reads for every live
/// task (the ordering keys, the runnable flag, and `name`, whose `String` holds
/// the niche the slab's occupied check reads), then what a poll needs, then the
/// timer, cancel and join bookkeeping that is touched rarely.
#[repr(C)]
struct TaskEntry<'env, R> {
    runnable_seq: u64,
    last_polled: u64,
    name: String,
    runnable: bool,
    state: TaskState,
    qos: QoSClass,
    cancel: Arc<CancelCell>,
    task: Option<Box<dyn ScopedTask<'env, R> + 'env>>,
    finish_waiters: Vec<TaskId>,
    sleeping_until: Option<Instant>,
    cancel_time_deadline: Option<Instant>,
    /// Never zero: it is the tick count plus a budget of at least one.
    cancel_tick_deadline: Option<NonZeroU64>,
}

struct SpawnReq<'env, R> {
    name: String,
    qos: QoSClass,
    task: Box<dyn ScopedTask<'env, R> + 'env>,
}

/// What a task asked for during its poll, applied once the poll returns. The
/// buffers live in the [`Scope`] so their capacity is reused from poll to poll;
/// they are empty between polls.
struct PollIntents<'env, R> {
    finish_subs: Vec<(TaskId, TaskId)>,
    spawns: Vec<SpawnReq<'env, R>>,
    sleeps: Vec<(TaskId, Instant)>,
    cancels: Vec<(TaskId, CancelEscalation, CancelReason)>,
}

// By hand: `#[derive(Default)]` would demand `R: Default`.
impl<'env, R> Default for PollIntents<'env, R> {
    fn default() -> Self {
        Self { finish_subs: Vec::new(), spawns: Vec::new(), sleeps: Vec::new(), cancels: Vec::new() }
    }
}

/// The next runnable sequence number. Spawns and wakes draw from one counter,
/// so `runnable_seq` orders tasks by when they last became runnable.
fn next_seq(counter: &mut u64) -> u64 {
    *counter = counter.wrapping_add(1);
    *counter
}

/// A runnable task's place under the active [`WakeOrdering`], read from its
/// entry once per round; smaller sorts first. The [`TaskId`] stored beside it
/// breaks ties in slot order, the order of the stable sort over every live
/// task this replaces.
type OrderKey = (u8, u64, u64);

fn order_key<R>(ordering: WakeOrdering, e: &TaskEntry<'_, R>) -> OrderKey {
    // `u8::MAX - priority` puts higher QoS first; `u64::MAX - seq` puts newer first.
    let qos = u8::MAX - e.qos.priority();
    match ordering {
        WakeOrdering::Fifo | WakeOrdering::RoundRobin => (0, 0, e.runnable_seq),
        WakeOrdering::Lifo => (0, 0, u64::MAX - e.runnable_seq),
        WakeOrdering::BiasedQoS => (qos, 0, e.runnable_seq),
        WakeOrdering::BiasedQoSThenAge => (qos, e.last_polled, e.runnable_seq),
    }
}

/// The runnable set, kept up to date as tasks change, so a round costs the
/// runnable tasks and not every live one.
///
/// A task enters (`enter`) when its `runnable` flag goes true: at spawn, and
/// when `drain_bus` takes a wake for an idle task (timer, finish-waiter and
/// cancel wakes all arrive that way). `reorder` sorts every member into `order`
/// for the round, which takes them from `order[next..]` and clears the flag of
/// each one it polls; that is how a polled task leaves. A task that finishes
/// leaves its stale id behind, and the next `reorder` drops it. Tasks the poll
/// budget did not reach stay in `order[next..]` and join the next `reorder`
/// with their `runnable_seq` unchanged.
///
/// Invariant: every live task whose `runnable` flag is set appears exactly
/// once in `entered` and `order[next..]` together, and no other live task does.
#[derive(Default)]
struct RunQueue {
    /// Tasks that became runnable since the last `reorder` (plus stale ids).
    entered: Vec<TaskId>,
    /// This round's order. Kept across rounds so its buffer is reused.
    order: Vec<(OrderKey, TaskId)>,
    /// The first entry of `order` the round has not taken.
    next: usize,
    /// The `RoundRobin` rotation, advanced once per round with a runnable task.
    rr_cursor: usize,
}

impl RunQueue {
    fn enter(&mut self, id: TaskId) {
        self.entered.push(id);
    }

    /// Order the whole runnable set for this round. `key` gives a member's
    /// sort key, or `None` when its id is stale or it is no longer runnable.
    fn reorder(&mut self, ordering: WakeOrdering, key: impl Fn(TaskId) -> Option<OrderKey>) {
        let RunQueue { entered, order, next, rr_cursor } = self;
        entered.extend(order.drain(*next..).map(|(_, id)| id));
        order.clear();
        *next = 0;
        order.extend(entered.drain(..).filter_map(|id| Some((key(id)?, id))));
        // Every id is distinct, so the order is total and unstable sorting is exact.
        order.sort_unstable();
        if ordering == WakeOrdering::RoundRobin && !order.is_empty() {
            let rotate = *rr_cursor % order.len();
            order.rotate_left(rotate);
            *rr_cursor = rr_cursor.wrapping_add(1);
        }
    }

    /// The next task in this round's order, if any is left.
    fn take_next(&mut self) -> Option<TaskId> {
        let &(_, id) = self.order.get(self.next)?;
        self.next += 1;
        Some(id)
    }
}

/// The runtime: the tasks, the wake bus, the timer and cancel heaps, and the
/// exclusive borrow of the environment `R` that every poll receives. Build one
/// with [`scoped`] or [`run_scoped`], or directly with [`Scope::new`].
pub struct Scope<'env, R> {
    resources: &'env mut R,
    slab: TaskSlab<TaskEntry<'env, R>>,
    bus: WakeBus,
    /// Wakes taken off the bus and not yet applied. `drain_bus` trades this
    /// (empty) buffer for the bus queue in one lock, then applies it unlocked.
    wake_batch: VecDeque<SchedulerMsg>,
    runq: RunQueue,

    sleep_heap: BinaryHeap<Reverse<(Instant, TaskId)>>,
    cancel_tick_heap: BinaryHeap<Reverse<(u64, TaskId)>>,
    cancel_time_heap: BinaryHeap<Reverse<(Instant, TaskId)>>,

    config: SchedulerConfig,
    tracer: Option<Box<dyn Tracer>>,
    intents: PollIntents<'env, R>,

    tick_count: u64,
    seq_counter: u64,
    poll_seq: u64,

    completed: usize,
    cancelled: usize,
    panicked: usize,
    rounds: u64,
    polls: u64,
}

/// Convenience: build a scope, hand it to `f`, return `f`'s value. Note this
/// does *not* run the scope — call `scope.run()` inside `f`, or use
/// [`run_scoped`] for the structured "spawn then join at the brace" form.
pub fn scoped<'env, R, Ret>(
    resources: &'env mut R,
    f: impl FnOnce(&mut Scope<'env, R>) -> Ret,
) -> Ret {
    let mut scope = Scope::new(resources);
    f(&mut scope)
}

/// Structured entry point: `build` spawns the root tasks, then every child is
/// driven to completion before this returns — the closing brace is the join
/// point, like `std::thread::scope`.
///
/// Tasks may borrow data living outside the scope (including `&mut`, held in
/// the task struct across suspensions):
///
/// ```
/// use scope::{run_scoped, Cx, FnTask, TaskPoll};
///
/// let mut env = ();
/// let outside = String::from("outlives the scope");
/// let r = &outside; // borrow of data that outlives the scope
/// run_scoped(&mut env, |s| {
///     s.spawn_named(
///         "ok",
///         FnTask(move |_cx: &mut Cx<'_, '_, ()>| {
///             let _ = r.len();
///             TaskPoll::Ready
///         }),
///     );
/// });
/// ```
///
/// The compiler enforces the bound: a task cannot borrow data that dies before
/// the scope finishes. The same task, borrowing data that dies first, is
/// rejected for exactly that reason (E0597, "does not live long enough"):
///
/// ```compile_fail,E0597
/// use scope::{run_scoped, Cx, FnTask, TaskPoll};
///
/// let mut env = ();
/// run_scoped(&mut env, |s| {
///     let local = String::from("dies when this closure returns");
///     let r = &local; // borrow of closure-local data
///     s.spawn_named(
///         "bad",
///         FnTask(move |_cx: &mut Cx<'_, '_, ()>| {
///             let _ = r.len(); // task would read it after `local` is dropped
///             TaskPoll::Ready
///         }),
///     );
/// }); // ERROR: `local` does not live long enough
/// ```
pub fn run_scoped<'env, R>(
    resources: &'env mut R,
    build: impl FnOnce(&mut Scope<'env, R>),
) -> RunReport {
    let mut scope = Scope::new(resources);
    build(&mut scope);
    scope.run()
}

impl<'env, R> Scope<'env, R> {
    /// A scope over `resources` with the default [`SchedulerConfig`].
    pub fn new(resources: &'env mut R) -> Self {
        Self::with_config(resources, SchedulerConfig::default())
    }

    /// A scope over `resources` with an explicit configuration.
    pub fn with_config(resources: &'env mut R, config: SchedulerConfig) -> Self {
        Self {
            resources,
            slab: TaskSlab::new(),
            bus: WakeBus::new(),
            wake_batch: VecDeque::new(),
            runq: RunQueue::default(),
            sleep_heap: BinaryHeap::new(),
            cancel_tick_heap: BinaryHeap::new(),
            cancel_time_heap: BinaryHeap::new(),
            config,
            tracer: None,
            intents: PollIntents::default(),
            tick_count: 0,
            seq_counter: 0,
            poll_seq: 0,
            completed: 0,
            cancelled: 0,
            panicked: 0,
            rounds: 0,
            polls: 0,
        }
    }

    /// Install a [`Tracer`]; the scope calls it at every scheduling event from now on.
    pub fn set_tracer(&mut self, tracer: Box<dyn Tracer>) {
        self.tracer = Some(tracer);
    }

    /// Remove the tracer and return it, for instance to read a [`crate::BufferTracer`].
    pub fn take_tracer(&mut self) -> Option<Box<dyn Tracer>> {
        self.tracer.take()
    }

    /// The configuration in effect.
    pub fn config(&self) -> SchedulerConfig {
        self.config
    }

    /// A real `Waker` that wakes `id` (reason `Io`). Hand this to external IO.
    #[cfg(not(loom))]
    pub fn waker_for(&self, id: TaskId, reason: WakeReason) -> std::task::Waker {
        crate::wake::scheduler_waker(id, reason, self.bus.clone())
    }

    /// Default tick budget for soft cancels requested from now on; 0 disables tick escalation.
    pub fn set_default_cancel_after_ticks(&mut self, ticks: u64) {
        self.config.default_cancel_ticks = ticks;
    }

    /// Default wall-clock budget for soft cancels requested from now on; `None` disables it.
    pub fn set_default_cancel_after_duration(&mut self, dur: Option<Duration>) {
        self.config.default_cancel_time = dur;
    }

    // ---------------- spawning ----------------

    fn insert_task(
        &mut self,
        name: String,
        qos: QoSClass,
        cell: Arc<CancelCell>,
        task: Box<dyn ScopedTask<'env, R> + 'env>,
    ) -> TaskId {
        let seq = next_seq(&mut self.seq_counter);
        let id = self.slab.insert(TaskEntry {
            runnable_seq: seq,
            last_polled: 0,
            name,
            runnable: true,
            state: TaskState::Alive,
            qos,
            cancel: cell,
            task: Some(task),
            finish_waiters: Vec::new(),
            sleeping_until: None,
            cancel_time_deadline: None,
            cancel_tick_deadline: None,
        });
        // The name moved into the entry; a tracer reads it from there.
        self.runq.enter(id);
        if let Some(t) = self.tracer.as_mut() {
            if let Some(e) = self.slab.get(id) {
                t.on_spawn(id, &e.name);
            }
        }
        id
    }

    /// Spawn `task` under `name` with the default QoS class; it runs on the next round.
    pub fn spawn_named<T>(&mut self, name: impl Into<String>, task: T) -> TaskId
    where
        T: ScopedTask<'env, R> + 'env,
    {
        let qos = self.config.default_qos;
        self.insert_task(name.into(), qos, Arc::new(CancelCell::new()), Box::new(task))
    }

    /// Spawn `task` under `name` with an explicit QoS class.
    pub fn spawn_named_qos<T>(&mut self, name: impl Into<String>, qos: QoSClass, task: T) -> TaskId
    where
        T: ScopedTask<'env, R> + 'env,
    {
        self.insert_task(name.into(), qos, Arc::new(CancelCell::new()), Box::new(task))
    }

    /// Spawn a task and get a handle to its eventual output.
    pub fn spawn_with_result<T, Out, F>(
        &mut self,
        name: impl Into<String>,
        task: T,
        finish: F,
    ) -> JoinHandle<Out>
    where
        T: ScopedTask<'env, R> + 'env,
        Out: 'env,
        F: FnOnce(&mut T, &mut R) -> Out + 'env,
    {
        let cell = Arc::new(CancelCell::new());
        let slot: Arc<Mutex<Option<Result<Out, JoinError>>>> = Arc::new(Mutex::new(None));
        let wrapper = OutputTask {
            inner: task,
            finish: Some(finish),
            slot: slot.clone(),
            cell: cell.clone(),
            sent: false,
        };
        let qos = self.config.default_qos;
        let id = self.insert_task(name.into(), qos, cell, Box::new(wrapper));
        JoinHandle::new(id, slot)
    }

    /// Spawn a task that completes after `dur`; join it to wait on the timer.
    pub fn sleep(&mut self, name: impl Into<String>, dur: Duration) -> JoinHandle<()> {
        self.spawn_with_result(name, SleepTask::new(dur), |_t, _r| ())
    }

    // ---------------- mailboxes ----------------

    /// Spawn a task that owns a typed [`Mailbox`]. `build` receives the read end
    /// to store in the task; the returned [`MailboxSender`] posts messages into
    /// it (from any task, or another thread) and wakes it.
    pub fn spawn_with_mailbox<M, T, F>(
        &mut self,
        name: impl Into<String>,
        build: F,
    ) -> (TaskId, MailboxSender<M>)
    where
        M: 'env,
        T: ScopedTask<'env, R> + 'env,
        F: FnOnce(Mailbox<M>) -> T,
    {
        let (queue, rx) = crate::mailbox::channel::<M>();
        let task = build(rx);
        let id = self.spawn_named(name, task);
        let tx = crate::mailbox::sender(queue, self.bus.clone(), id);
        (id, tx)
    }

    /// Like [`spawn_with_mailbox`](Self::spawn_with_mailbox) but also yields a
    /// [`JoinHandle`] for the task's output.
    pub fn spawn_with_mailbox_result<M, T, Out, F, Fin>(
        &mut self,
        name: impl Into<String>,
        build: F,
        finish: Fin,
    ) -> (JoinHandle<Out>, MailboxSender<M>)
    where
        M: 'env,
        T: ScopedTask<'env, R> + 'env,
        Out: 'env,
        F: FnOnce(Mailbox<M>) -> T,
        Fin: FnOnce(&mut T, &mut R) -> Out + 'env,
    {
        let (queue, rx) = crate::mailbox::channel::<M>();
        let task = build(rx);
        let handle = self.spawn_with_result(name, task, finish);
        let tx = crate::mailbox::sender(queue, self.bus.clone(), handle.task);
        (handle, tx)
    }

    // ---------------- combinators ----------------

    /// Spawn a task that waits for every handle and yields all their results in order.
    pub fn spawn_join_all<T>(
        &mut self,
        name: impl Into<String>,
        joins: Vec<JoinHandle<T>>,
    ) -> JoinHandle<Vec<Result<T, JoinError>>>
    where
        T: 'env,
    {
        let (setter, slot) = result_channel();
        let producers = joins.iter().map(|j| j.task).collect::<Vec<_>>();
        let n = joins.len();
        let task = JoinAllTask {
            producers,
            joins,
            results: (0..n).map(|_| None).collect(),
            subscribed: vec![false; n],
            out: setter,
        };
        let id = self.spawn_named(name, task);
        JoinHandle::new(id, slot)
    }

    /// Like [`Scope::spawn_join_all`] for fallible tasks: every `Ok` value in order, or the first failure.
    pub fn spawn_join_all_ok<T, E>(
        &mut self,
        name: impl Into<String>,
        joins: Vec<JoinHandle<Result<T, E>>>,
    ) -> JoinHandle<Result<Vec<T>, JoinAllOkError<E>>>
    where
        T: 'env,
        E: 'env,
    {
        let (setter, slot) = result_channel();
        let producers = joins.iter().map(|j| j.task).collect::<Vec<_>>();
        let n = joins.len();
        let task = JoinAllOkTask {
            producers,
            joins,
            subscribed: vec![false; n],
            vals: (0..n).map(|_| None).collect(),
            out: setter,
        };
        let id = self.spawn_named(name, task);
        JoinHandle::new(id, slot)
    }

    /// Spawn a task that yields whichever of `a` or `b` finishes first, as an [`Either`].
    pub fn spawn_select2<A, B>(
        &mut self,
        name: impl Into<String>,
        a: JoinHandle<A>,
        b: JoinHandle<B>,
    ) -> JoinHandle<Either<Result<A, JoinError>, Result<B, JoinError>>>
    where
        A: 'env,
        B: 'env,
    {
        let (setter, slot) = result_channel();
        let task = Select2Task {
            prod_a: a.task,
            prod_b: b.task,
            a: Some(a),
            b: Some(b),
            sub_a: false,
            sub_b: false,
            flip: false,
            out: setter,
        };
        let id = self.spawn_named(name, task);
        JoinHandle::new(id, slot)
    }

    /// Spawn a task that yields the position and result of the first handle to finish.
    pub fn spawn_select_n<T>(
        &mut self,
        name: impl Into<String>,
        joins: Vec<JoinHandle<T>>,
    ) -> JoinHandle<(usize, Result<T, JoinError>)>
    where
        T: 'env,
    {
        let (setter, slot) = result_channel();
        let producers = joins.iter().map(|j| j.task).collect::<Vec<_>>();
        let n = joins.len();
        let task = SelectNTask {
            producers,
            joins,
            subscribed: vec![false; n],
            rr: 0,
            out: setter,
        };
        let id = self.spawn_named(name, task);
        JoinHandle::new(id, slot)
    }

    /// Spawn a task that waits for `join` and applies `f` to its result.
    pub fn spawn_map_join<T, Out, F>(
        &mut self,
        name: impl Into<String>,
        join: JoinHandle<T>,
        f: F,
    ) -> JoinHandle<Out>
    where
        T: 'env,
        Out: 'env,
        F: for<'p> FnOnce(Result<T, JoinError>, &mut Cx<'p, 'env, R>) -> Out + 'env,
    {
        let (setter, slot) = result_channel();
        let producer = join.task;
        let task = MapJoinTask {
            producer,
            join: Some(join),
            f: Some(f),
            subscribed: false,
            out: setter,
        };
        let id = self.spawn_named(name, task);
        JoinHandle::new(id, slot)
    }

    // ---------------- external cancellation ----------------

    /// Hard-stop `id` right now with `reason` (safe to call between rounds).
    pub fn cancel_now_with_reason(&mut self, id: TaskId, reason: CancelReason) {
        let Some(e) = self.slab.get(id).filter(|e| e.state == TaskState::Alive) else { return };
        e.cancel.request(reason);
        if let Some(t) = self.tracer.as_mut() {
            t.on_cancel_hard_stop(id, reason);
        }
        self.finish_task(id, TaskState::Cancelled);
    }

    /// Hard-stop `id` immediately with reason `UserRequest`.
    pub fn cancel_now(&mut self, id: TaskId) {
        self.cancel_now_with_reason(id, CancelReason::UserRequest);
    }

    /// Best-effort cooperative cancel with the scope's default escalation.
    pub fn request_cancel(&mut self, id: TaskId, reason: CancelReason) {
        let Some(e) = self.slab.get(id).filter(|e| e.state == TaskState::Alive) else { return };
        e.cancel.request(reason);
        self.bus.wake(id, WakeReason::Cancel);
        self.schedule_cancel_deadlines(id, CancelEscalation::DEFAULT, reason);
    }

    // ---------------- driving ----------------

    /// One scheduler round: fire due timers/cancels, drain wakes, then poll the
    /// runnable set once in policy order.
    pub fn tick(&mut self) -> TickOutcome {
        if self.slab.is_empty() {
            return TickOutcome::Done;
        }
        let polled = self.round();
        if self.slab.is_empty() {
            TickOutcome::Done
        } else if polled == 0 {
            TickOutcome::Idle
        } else {
            TickOutcome::Progress(polled)
        }
    }

    /// Drive to completion (or stall) and report. Never hangs on pure compute:
    /// an idle round with no deadlines and nothing runnable ends the run.
    pub fn run(&mut self) -> RunReport {
        loop {
            let polled = self.round();
            if self.slab.is_empty() {
                break;
            }
            if polled == 0 {
                match self.next_deadline() {
                    // Sleep until the deadline or a wake from another thread,
                    // whichever comes first; the wake stays queued for the
                    // next round's drain.
                    #[cfg(not(loom))]
                    Some(wait) => self.bus.wait_timeout(wait),
                    // No deadline (and no wall clock under loom): only a wake
                    // already queued can make progress.
                    _ if self.bus.is_empty() => break,
                    _ => {}
                }
            }
        }
        self.build_report()
    }

    /// Drive until completion or `timeout`. Returns `true` if everything
    /// finished.
    #[cfg(not(loom))]
    pub fn run_until(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let polled = self.round();
            if self.slab.is_empty() {
                return true;
            }
            if polled == 0 {
                if Instant::now() >= deadline {
                    return false;
                }
                let remain = deadline.saturating_duration_since(Instant::now());
                let wait = match self.next_deadline() {
                    Some(t) => t.min(remain),
                    None => remain,
                };
                // Returns at once if a wake is queued; never takes it off the bus.
                self.bus.wait_timeout(wait);
            }
        }
    }

    fn round(&mut self) -> usize {
        self.tick_count = self.tick_count.wrapping_add(1);
        self.rounds += 1;
        if let Some(t) = self.tracer.as_mut() {
            t.on_round_begin(self.tick_count);
        }

        self.fire_expired_cancel_ticks();
        self.fire_expired_cancel_times();
        self.fire_expired_timers();
        self.drain_bus();

        let budget = self.config.max_polls_per_round;
        let ordering = self.config.wake_ordering;
        #[cfg(test)]
        let expected = self.full_scan_order(self.runq.rr_cursor);
        let slab = &self.slab;
        self.runq.reorder(ordering, |id| {
            slab.get(id)
                .filter(|e| e.runnable && e.state == TaskState::Alive)
                .map(|e| order_key(ordering, e))
        });
        #[cfg(test)]
        assert!(
            self.runq.order.iter().map(|&(_, id)| id).eq(expected.iter().copied()),
            "round {}: the runnable set's order differs from a full scan",
            self.tick_count
        );
        let mut polled = 0;
        // Past the budget, the rest stay in `runq` and wait for a later round.
        while polled < budget {
            let Some(id) = self.runq.take_next() else { break };
            match self.slab.get_mut(id) {
                Some(e) if e.runnable && e.state == TaskState::Alive => e.runnable = false,
                _ => continue,
            }
            self.poll_one(id);
            polled += 1;
        }
        polled
    }

    /// Apply every queued wake, oldest first: one lock takes the whole queue,
    /// then the batch is applied with the lock released.
    fn drain_bus(&mut self) {
        loop {
            // A batch cut short by a panicking tracer is still here, ahead of
            // anything sent since; `take_all` appends behind it.
            self.bus.take_all(&mut self.wake_batch);
            if self.wake_batch.is_empty() {
                return;
            }
            self.apply_wake_batch();
            // A tracer hook is the only code on this thread that can send while
            // a batch is applied; look again so its wakes land this round, as
            // they did when every wake was popped one lock at a time. With no
            // tracer, only another thread can have sent since the take, and
            // that wake is just as valid next round.
            if self.tracer.is_none() {
                return;
            }
        }
    }

    fn apply_wake_batch(&mut self) {
        while let Some(SchedulerMsg::Wake { id, reason }) = self.wake_batch.pop_front() {
            if let Some(t) = self.tracer.as_mut() {
                t.on_wake(id, reason);
            }
            if let Some(e) = self.slab.get_mut(id) {
                if e.state == TaskState::Alive && !e.runnable {
                    e.runnable = true;
                    e.runnable_seq = next_seq(&mut self.seq_counter);
                    self.runq.enter(id);
                }
            }
        }
    }

    fn poll_one(&mut self, id: TaskId) {
        let mut tracer = self.tracer.take();
        self.poll_seq += 1;
        self.polls += 1;
        let seq = self.poll_seq;

        // One lookup to announce the poll, stamp it and take the task out. The
        // name is only read when a tracer is attached, and then only borrowed.
        let Some(e) = self.slab.get_mut(id) else {
            self.tracer = tracer;
            return;
        };
        if let Some(t) = tracer.as_mut() {
            t.on_poll_begin(seq, id, &e.name);
        }
        e.last_polled = self.tick_count;
        let Some(mut task) = e.task.take() else {
            self.tracer = tracer;
            return;
        };

        // Intents recorded during poll, applied after — keeps every hook off of
        // `&mut self.slab` so no `unsafe` is needed to satisfy the borrow checker.
        // The buffers are lent out of the scope for this poll and handed back
        // empty, so their capacity carries over to the next poll.
        let mut intents = std::mem::take(&mut self.intents);

        let res = {
            let slab = &self.slab;
            let bus = &self.bus;
            // The entry is still in the slab: nothing removes one while the slab
            // is borrowed shared, as it is for the whole poll. `Cx` borrows the
            // entry's cancel cell from here, and the tracer its name.
            let Some(entry) = slab.get(id) else {
                self.tracer = tracer;
                self.intents = intents;
                return;
            };
            let PollIntents { finish_subs, spawns, sleeps, cancels } = &mut intents;

            let mut subscribe_finish =
                |waiter: TaskId, target: TaskId| finish_subs.push((waiter, target));
            let mut defer_spawn = |_parent: TaskId,
                                   name: String,
                                   qos: QoSClass,
                                   task: Box<dyn ScopedTask<'env, R> + 'env>| {
                spawns.push(SpawnReq { name, qos, task });
            };
            let mut query_state =
                |qid: TaskId| slab.get(qid).map(|e| e.state).unwrap_or(TaskState::Completed);
            let mut subscribe_sleep_until = |tid: TaskId, dl: Instant| sleeps.push((tid, dl));
            let mut request_cancel =
                |target: TaskId, esc: CancelEscalation, reason: CancelReason| {
                    if let Some(e) = slab.get(target) {
                        if e.state == TaskState::Alive {
                            e.cancel.request(reason);
                            bus.wake(target, WakeReason::Cancel);
                        }
                    }
                    cancels.push((target, esc, reason));
                };

            let mut cx = Cx {
                resources: &mut *self.resources,
                id,
                bus,
                cancel: &entry.cancel,
                subscribe_finish: &mut subscribe_finish,
                defer_spawn: &mut defer_spawn,
                query_state: &mut query_state,
                subscribe_sleep_until: &mut subscribe_sleep_until,
                request_cancel: &mut request_cancel,
            };

            let res = catch_unwind(AssertUnwindSafe(|| task.poll(&mut cx)));

            if let Some(t) = tracer.as_mut() {
                t.on_poll_end(id, &entry.name);
            }
            res
        };
        self.tracer = tracer;

        // Apply recorded intents.
        for (waiter, target) in intents.finish_subs.drain(..) {
            match self.slab.get_mut(target) {
                Some(e) => e.finish_waiters.push(waiter),
                None => self.bus.wake(waiter, WakeReason::Join),
            }
        }
        for (tid, dl) in intents.sleeps.drain(..) {
            if let Some(e) = self.slab.get_mut(tid) {
                e.sleeping_until = Some(dl);
                self.sleep_heap.push(Reverse((dl, tid)));
            }
        }
        for req in intents.spawns.drain(..) {
            if let Some(t) = self.tracer.as_mut() {
                t.on_defer_spawn(id, &req.name);
            }
            self.insert_task(req.name, req.qos, Arc::new(CancelCell::new()), req.task);
        }
        for (target, esc, reason) in intents.cancels.drain(..) {
            self.schedule_cancel_deadlines(target, esc, reason);
        }
        self.intents = intents;

        // Handle the poll result.
        match res {
            Ok(TaskPoll::Pending) => {
                if let Some(e) = self.slab.get_mut(id) {
                    if e.state == TaskState::Alive {
                        e.task = Some(task);
                    }
                }
            }
            Ok(TaskPoll::Ready) => {
                let cancelled =
                    self.slab.get(id).map(|e| e.cancel.is_cancelled()).unwrap_or(false);
                drop(task);
                let state = if cancelled { TaskState::Cancelled } else { TaskState::Completed };
                self.finish_task(id, state);
            }
            Err(_) => {
                drop(task);
                self.finish_task(id, TaskState::Panicked);
            }
        }
    }

    fn finish_task(&mut self, id: TaskId, state: TaskState) {
        let Some(mut entry) = self.slab.remove(id) else { return };
        entry.state = state;
        // Drop the task box now so its result slot is filled before waiters run.
        entry.task = None;

        match state {
            TaskState::Completed => self.completed += 1,
            TaskState::Cancelled => self.cancelled += 1,
            TaskState::Panicked => self.panicked += 1,
            TaskState::Alive => {}
        }
        if let Some(t) = self.tracer.as_mut() {
            t.on_finish(id, &entry.name, state);
        }
        for waiter in std::mem::take(&mut entry.finish_waiters) {
            self.bus.wake(waiter, WakeReason::Join);
        }
    }

    // ---------------- cancellation bookkeeping ----------------

    fn schedule_cancel_deadlines(
        &mut self,
        target: TaskId,
        esc: CancelEscalation,
        reason: CancelReason,
    ) {
        let Some(e) = self.slab.get_mut(target).filter(|e| e.state == TaskState::Alive) else {
            return;
        };
        let budgets = resolve_cancel_budgets(
            self.config.default_cancel_ticks,
            self.config.default_cancel_time,
            esc,
        );
        if let Some(t) = self.tracer.as_mut() {
            t.on_cancel_request(
                target,
                reason,
                if budgets.tick_budget > 0 { Some(budgets.tick_budget) } else { None },
                budgets.time_budget.filter(|d| *d != Duration::ZERO),
            );
        }
        if budgets.immediate {
            let dl = Instant::now();
            e.cancel_time_deadline = Some(dl);
            self.cancel_time_heap.push(Reverse((dl, target)));
            return;
        }
        if budgets.tick_budget > 0 {
            let dl = self.tick_count.saturating_add(budgets.tick_budget);
            e.cancel_tick_deadline = NonZeroU64::new(dl);
            self.cancel_tick_heap.push(Reverse((dl, target)));
        }
        if let Some(dur) = budgets.time_budget {
            let dl = Instant::now() + dur;
            e.cancel_time_deadline = Some(dl);
            self.cancel_time_heap.push(Reverse((dl, target)));
        }
    }

    fn fire_expired_cancel_ticks(&mut self) {
        while let Some(&Reverse((dl, id))) = self.cancel_tick_heap.peek() {
            if dl > self.tick_count {
                break;
            }
            self.cancel_tick_heap.pop();
            let fire = self
                .slab
                .get(id)
                .map(|e| {
                    e.state == TaskState::Alive
                        && e.cancel_tick_deadline.map(NonZeroU64::get) == Some(dl)
                        && e.cancel.is_cancelled()
                })
                .unwrap_or(false);
            if fire {
                self.cancel_now_with_reason(id, CancelReason::TimeoutTicks);
            }
        }
    }

    fn fire_expired_cancel_times(&mut self) {
        // Read the clock only when there is a deadline to compare it with.
        if self.cancel_time_heap.is_empty() {
            return;
        }
        let now = Instant::now();
        while let Some(&Reverse((dl, id))) = self.cancel_time_heap.peek() {
            if dl > now {
                break;
            }
            self.cancel_time_heap.pop();
            let fire = self
                .slab
                .get(id)
                .map(|e| {
                    e.state == TaskState::Alive
                        && e.cancel_time_deadline == Some(dl)
                        && e.cancel.is_cancelled()
                })
                .unwrap_or(false);
            if fire {
                self.cancel_now_with_reason(id, CancelReason::TimeoutTime);
            }
        }
    }

    fn fire_expired_timers(&mut self) {
        // Read the clock only when there is a deadline to compare it with.
        if self.sleep_heap.is_empty() {
            return;
        }
        let now = Instant::now();
        while let Some(&Reverse((dl, id))) = self.sleep_heap.peek() {
            if dl > now {
                break;
            }
            self.sleep_heap.pop();
            match self.slab.get_mut(id) {
                Some(e) if e.state == TaskState::Alive && e.sleeping_until == Some(dl) => {
                    e.sleeping_until = None;
                }
                _ => continue,
            }
            self.bus.wake(id, WakeReason::Timer);
        }
    }

    fn next_deadline(&self) -> Option<Duration> {
        let next_sleep = self.sleep_heap.peek().map(|Reverse((dl, _))| *dl);
        let next_cancel = self.cancel_time_heap.peek().map(|Reverse((dl, _))| *dl);
        let next = match (next_sleep, next_cancel) {
            (None, None) => return None,
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (Some(a), Some(b)) => a.min(b),
        };
        Some(next.saturating_duration_since(Instant::now()))
    }

    fn build_report(&mut self) -> RunReport {
        let stalled = self.slab.entries().map(|(id, e)| (id, e.name.clone())).collect();
        RunReport {
            rounds: self.rounds,
            polls: self.polls,
            completed: self.completed,
            cancelled: self.cancelled,
            panicked: self.panicked,
            stalled,
        }
    }
}

// ---------------- output wrapper ----------------

/// Wraps a user task, capturing its output into a join slot. Fills
/// `Err(Cancelled)` when it observes cancellation, and `Err(...)` on drop if it
/// ended without a value (panic or hard-stop).
struct OutputTask<T, Out, F> {
    inner: T,
    finish: Option<F>,
    slot: Arc<Mutex<Option<Result<Out, JoinError>>>>,
    cell: Arc<CancelCell>,
    sent: bool,
}

impl<'env, R, T, Out, F> ScopedTask<'env, R> for OutputTask<T, Out, F>
where
    T: ScopedTask<'env, R> + 'env,
    Out: 'env,
    F: FnOnce(&mut T, &mut R) -> Out + 'env,
{
    fn poll(&mut self, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
        if cx.is_cancelled() && !self.sent {
            *lock(&self.slot) = Some(Err(JoinError::Cancelled(cx.cancel_reason())));
            self.sent = true;
            return TaskPoll::Ready;
        }
        match self.inner.poll(cx) {
            TaskPoll::Pending => TaskPoll::Pending,
            TaskPoll::Ready => {
                if let Some(f) = self.finish.take() {
                    let out = f(&mut self.inner, cx.resources);
                    if !self.sent {
                        *lock(&self.slot) = Some(Ok(out));
                        self.sent = true;
                    }
                }
                TaskPoll::Ready
            }
        }
    }
}

impl<T, Out, F> Drop for OutputTask<T, Out, F> {
    fn drop(&mut self) {
        if self.sent {
            return;
        }
        let err = if self.cell.is_cancelled() {
            JoinError::Cancelled(self.cell.reason())
        } else {
            JoinError::Panicked
        };
        *lock(&self.slot) = Some(Err(err));
    }
}

// ---------------- tests ----------------

#[cfg(test)]
impl<'env, R> Scope<'env, R> {
    /// The order every round used before the runnable set was kept up to date:
    /// scan every live task, keep the runnable ones in slot order, then stable
    /// sort them with comparators that read the slab. `round` checks its own
    /// order against this under `cfg(test)`.
    fn full_scan_order(&self, rr_cursor: usize) -> Vec<TaskId> {
        let mut ids: Vec<TaskId> = self
            .slab
            .live_ids()
            .into_iter()
            .filter(|id| {
                self.slab
                    .get(*id)
                    .map(|e| e.runnable && e.state == TaskState::Alive)
                    .unwrap_or(false)
            })
            .collect();
        let seq = |id: TaskId| self.slab.get(id).map(|e| e.runnable_seq).unwrap_or(0);
        let qos = |id: TaskId| self.slab.get(id).map(|e| e.qos.priority()).unwrap_or(0);
        let last = |id: TaskId| self.slab.get(id).map(|e| e.last_polled).unwrap_or(0);
        match self.config.wake_ordering {
            WakeOrdering::Fifo => ids.sort_by_key(|a| seq(*a)),
            WakeOrdering::Lifo => ids.sort_by_key(|a| Reverse(seq(*a))),
            WakeOrdering::RoundRobin => {
                ids.sort_by_key(|a| seq(*a));
                if !ids.is_empty() {
                    let rotate = rr_cursor % ids.len();
                    ids.rotate_left(rotate);
                }
            }
            WakeOrdering::BiasedQoS => ids.sort_by(|a, b| {
                qos(*b).cmp(&qos(*a)).then_with(|| seq(*a).cmp(&seq(*b)))
            }),
            WakeOrdering::BiasedQoSThenAge => ids.sort_by(|a, b| {
                qos(*b)
                    .cmp(&qos(*a))
                    .then_with(|| last(*a).cmp(&last(*b)))
                    .then_with(|| seq(*a).cmp(&seq(*b)))
            }),
        }
        ids
    }
}

// A whole Scope outside a loom model; the loom build checks the bus on its own.
#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    const CLASSES: [QoSClass; 5] = [
        QoSClass::Background,
        QoSClass::Batch,
        QoSClass::Standard,
        QoSClass::Interactive,
        QoSClass::Realtime,
    ];

    /// A deterministic pseudo-random stream (64-bit LCG).
    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, n: u64) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (self.0 >> 33) % n.max(1)
        }
    }

    /// What the tasks share: the random stream and every task id seen so far
    /// (finished ones included, so wakes and cancels also hit stale ids).
    struct Env {
        rng: Lcg,
        ids: Vec<TaskId>,
        polls: u64,
    }

    impl Env {
        fn any_id(&mut self) -> TaskId {
            let i = self.rng.below(self.ids.len() as u64) as usize;
            self.ids[i]
        }
    }

    /// Does one random thing per poll. Many branches leave it idle until some
    /// other task wakes it, so most rounds have few runnable tasks among many.
    struct Chaos {
        left: u32,
        registered: bool,
    }

    impl<'env> ScopedTask<'env, Env> for Chaos {
        fn poll(&mut self, cx: &mut Cx<'_, 'env, Env>) -> TaskPoll {
            cx.resources.polls += 1;
            if !self.registered {
                self.registered = true;
                let id = cx.id();
                if !cx.resources.ids.contains(&id) {
                    cx.resources.ids.push(id);
                }
            }
            if self.left == 0 || cx.is_cancelled() {
                return TaskPoll::Ready;
            }
            self.left -= 1;
            match cx.resources.rng.below(9) {
                0 | 1 => return cx.yield_now(),
                2 => {
                    let t = cx.resources.any_id();
                    cx.wake(t);
                }
                3 => {
                    if cx.resources.ids.len() < 120 {
                        let qos = CLASSES[cx.resources.rng.below(5) as usize];
                        cx.spawn_later_qos("child", qos, Chaos { left: 3, registered: false });
                    }
                    cx.wake_self();
                }
                4 => {
                    let t = cx.resources.any_id();
                    cx.cancel(t);
                }
                5 => {
                    let t = cx.resources.any_id();
                    cx.subscribe_to_finish(t);
                }
                6 => cx.sleep_for(Duration::ZERO),
                7 => {
                    let t = cx.resources.any_id();
                    cx.wake(t);
                    cx.wake_self();
                }
                _ => {} // idle until someone wakes it
            }
            TaskPoll::Pending
        }
    }

    /// The runnable set must order every round exactly as a full scan of every
    /// live task would, under every ordering and poll budget, through spawns,
    /// wakes (also of finished tasks), timers, finish waiters, soft cancels
    /// that escalate by tick, and hard cancels between rounds.
    #[test]
    fn runnable_set_orders_like_a_full_scan() {
        // Miri runs one small workload per ordering and budget; natively, six full ones.
        let (runs, roots_n, max_rounds, min_polls) =
            if cfg!(miri) { (1, 10, 60, 100) } else { (6, 24, 400, 1000) };
        let mut total_polls = 0;
        for ordering in WakeOrdering::ALL {
            for budget in [usize::MAX, 1, 3] {
                for run in 0..runs {
                    let seed = run * 7919 + budget.min(100) as u64;
                    let mut env = Env { rng: Lcg(seed), ids: Vec::new(), polls: 0 };
                    let mut outside = Lcg(run + 17);
                    {
                        let config =
                            SchedulerConfig::with_ordering(ordering).with_poll_budget(budget);
                        let mut scope = Scope::with_config(&mut env, config);
                        scope.set_default_cancel_after_ticks(2);
                        let mut roots = Vec::new();
                        for _ in 0..roots_n {
                            let qos = CLASSES[outside.below(5) as usize];
                            let root = Chaos { left: 8, registered: false };
                            roots.push(scope.spawn_named_qos("root", qos, root));
                        }
                        let mut idle = 0;
                        for _ in 0..max_rounds {
                            match scope.tick() {
                                TickOutcome::Done => break,
                                TickOutcome::Idle => idle += 1,
                                TickOutcome::Progress(_) => idle = 0,
                            }
                            if idle > 3 {
                                break;
                            }
                            let target = roots[outside.below(roots.len() as u64) as usize];
                            match outside.below(12) {
                                0 => scope.cancel_now(target),
                                1 => scope.request_cancel(target, CancelReason::UserRequest),
                                _ => {}
                            }
                        }
                    }
                    total_polls += env.polls;
                }
            }
        }
        assert!(total_polls > min_polls, "the workload barely ran: {total_polls} polls");
    }
}
