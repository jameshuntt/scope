// src/scope.rs
use crate::{
    CancelEscalation,
    CancelReason,
    TickEscalation,
    TimeEscalation,
    io::IoToken,
    ops::{
        all::{JoinAllOkError, JoinAllOkTask, JoinAllTask},
        map::MapJoinTask,
    },
    select::{Either, Select2Task, SelectNTask},
    task::{Cx, EXTERNAL_WAKE, JoinError, JoinPoll, ScopedTask, TaskId, TaskPoll, TaskState, WakeMsg, YieldNowTask},
    trace::Tracer,
    wake::{wake_channel, WakeRecvTimeout, WakeRx, WakeTx},
};
use std::{
    cmp::Reverse, collections::{BinaryHeap, VecDeque}, panic::{AssertUnwindSafe, catch_unwind}, pin::Pin, ptr::NonNull, sync::{
        Arc, atomic::{AtomicBool, AtomicU8, Ordering}, mpsc
    }, time::{Duration, Instant}
};

// ----------------------------- tick result -----------------------------
pub enum TickResult {
    Progress,
    Idle,
    Done,
}

// Reserve u8::MAX as “panicked” sentinel in abort_reason.
const ABORT_PANICKED: u8 = u8::MAX;

// ----------------------------- cancel budgets -----------------------------
#[derive(Copy, Clone, Debug)]
struct CancelBudgets {
    tick_budget: u64,
    time_budget: Option<Duration>,
    immediate: bool, // "hard stop now" (or "schedule now" in poll hook)
}

#[inline]
fn resolve_cancel_budgets(
    default_tick: u64,
    default_time: Option<Duration>,
    esc: CancelEscalation,
) -> CancelBudgets {
    let tick_budget = match esc.tick {
        TickEscalation::Default => default_tick,
        TickEscalation::None => 0,
        TickEscalation::After(n) => n,
    };

    let time_budget = match esc.time {
        TimeEscalation::Default => default_time,
        TimeEscalation::None => None,
        TimeEscalation::After(d) => Some(d),
        TimeEscalation::Immediate => Some(Duration::ZERO),
    };

    let immediate =
        matches!(esc.time, TimeEscalation::Immediate) || time_budget == Some(Duration::ZERO);

    CancelBudgets {
        tick_budget,
        time_budget,
        immediate,
    }
}

// ----------------------------- JoinHandle -----------------------------
/// Canonical JoinHandle: always yields Result<T, JoinError>.
pub struct JoinHandle<T> {
    pub task: TaskId,
    rx: mpsc::Receiver<Result<T, JoinError>>,
}
impl<T> JoinHandle<T> {
    #[inline]
    pub fn poll(&mut self) -> JoinPoll<T> {
        match self.rx.try_recv() {
            Ok(v) => JoinPoll::Ready(v),
            Err(mpsc::TryRecvError::Empty) => JoinPoll::Pending,
            Err(mpsc::TryRecvError::Disconnected) => JoinPoll::Ready(Err(JoinError::Panicked)),
        }
    }

    #[inline]
    pub fn recv(self) -> Result<T, JoinError> {
        self.rx.recv().unwrap_or(Err(JoinError::Panicked))
    }
}

struct SpawnReq<'env, R> {
    parent: TaskId,
    name: String,
    task: Pin<Box<dyn ScopedTask<'env, R> + 'env>>,
}

struct TaskEntry<'env, R> {
    name: String,
    state: TaskState,
    task: Option<Pin<Box<dyn ScopedTask<'env, R> + 'env>>>,
}

// ---------------- scope ----------------

pub struct Scope<'env, R> {
    resources: &'env mut R,

    tasks: Vec<TaskEntry<'env, R>>,
    active: usize,

    ready: VecDeque<TaskId>,
    queued: Vec<bool>,

    wake_tx: WakeTx,
    wake_rx: WakeRx,

    finish_waiters: Vec<Vec<TaskId>>,
    spawns: VecDeque<SpawnReq<'env, R>>,

    io_read_waiters: Vec<Vec<TaskId>>,

    // Timer wheel:
    sleeping_until: Vec<Option<Instant>>,
    sleep_heap: BinaryHeap<Reverse<(Instant, TaskId)>>,

    // Cancellation:
    cancel_flags: Vec<Arc<AtomicBool>>,
    cancel_reason: Vec<u8>, // CancelReason::as_u8()
    abort_reason: Vec<Option<Arc<AtomicU8>>>, // 0 none, u8::MAX panicked, else CancelReason code.

    tick_count: u64,
    cancel_budget_default: u64,
    cancel_deadline: Vec<Option<u64>>,
    cancel_heap: BinaryHeap<Reverse<(u64, TaskId)>>,

    cancel_time_default: Option<Duration>,
    cancel_time_deadline: Vec<Option<Instant>>,
    cancel_time_heap: BinaryHeap<Reverse<(Instant, TaskId)>>,

    poll_seq: u64,
    tracer: Option<Box<dyn Tracer>>,
}

pub fn scoped<'env, R, Ret>(
    resources: &'env mut R,
    f: impl FnOnce(&mut Scope<'env, R>) -> Ret,
) -> Ret {
    let mut scope = Scope::new(resources);
    f(&mut scope)
}

impl<'env, R> Scope<'env, R> {
    pub fn new(resources: &'env mut R) -> Self {
        let (wake_tx, wake_rx) = wake_channel();
        Self {
            resources,

            tasks: Vec::new(),
            active: 0,

            ready: VecDeque::new(),
            queued: Vec::new(),

            wake_tx,
            wake_rx,

            finish_waiters: Vec::new(),
            spawns: VecDeque::new(),
            io_read_waiters: Vec::new(),

            sleeping_until: Vec::new(),
            sleep_heap: BinaryHeap::new(),

            cancel_flags: Vec::new(),
            cancel_reason: Vec::new(),
            abort_reason: Vec::new(),
            tick_count: 0,

            cancel_budget_default: 0,
            cancel_deadline: Vec::new(),
            cancel_heap: BinaryHeap::new(),

            cancel_time_default: None,
            cancel_time_deadline: Vec::new(),
            cancel_time_heap: BinaryHeap::new(),

            poll_seq: 0,
            tracer: None,
        }
    }

    pub fn set_tracer(&mut self, tracer: Box<dyn Tracer>) {
        self.tracer = Some(tracer);
    }

    // ---------------- unified task slot init/push ----------------

    #[inline]
    fn init_new_task_slot(&mut self) {
        self.queued.push(false);
        self.finish_waiters.push(Vec::new());
        self.sleeping_until.push(None);

        self.cancel_flags.push(Arc::new(AtomicBool::new(false)));
        self.cancel_reason.push(CancelReason::None.as_u8());
        self.abort_reason.push(None);

        self.cancel_deadline.push(None);
        self.cancel_time_deadline.push(None);
    }

    #[inline]
    fn push_task_dyn(
        &mut self,
        name: String,
        task: Pin<Box<dyn ScopedTask<'env, R> + 'env>>,
    ) -> TaskId {
        let id = self.tasks.len();

        self.tasks.push(TaskEntry {
            name: name.clone(),
            state: TaskState::Alive,
            task: Some(task),
        });

        self.active += 1;
        self.init_new_task_slot();

        if let Some(t) = self.tracer.as_mut() {
            t.on_spawn(id, &name);
        }

        self.enqueue_ready(id);
        id
    }

    // ---------------- IO integration ----------------

    fn ensure_io_len(&mut self, token: IoToken) {
        let t = token as usize;
        if t >= self.io_read_waiters.len() {
            self.io_read_waiters.resize_with(t + 1, Vec::new);
        }
    }

    pub fn notify_readable(&mut self, token: IoToken) {
        let t = token as usize;
        if t >= self.io_read_waiters.len() {
            return;
        }

        self.ensure_io_len(token);

        let woken = self.io_read_waiters[t].drain(..).collect::<Vec<_>>();
        if let Some(tr) = self.tracer.as_mut() {
            tr.on_io_notify_readable(token, &woken);
        }
        for task in woken {
            self.wake_tx.send(WakeMsg {
                from: EXTERNAL_WAKE,
                to: task,
            });
        }
    }

    pub fn sleep(&mut self, name: impl Into<String>, dur: Duration) -> JoinHandle<()> {
        use crate::timer::SleepTask;
        self.spawn_with_result(name, SleepTask::new(dur), |_t, _r| ())
    }

    // ---------------- cancellation defaults ----------------

    pub fn set_default_cancel_after_ticks(&mut self, ticks: usize) {
        self.cancel_budget_default = ticks as u64;
    }

    pub fn set_default_cancel_after_duration(&mut self, dur: Option<Duration>) {
        self.cancel_time_default = dur;
    }

    // ---------------- spawning ----------------

    pub fn spawn_named<T>(&mut self, name: impl Into<String>, task: T) -> TaskId
    where
        T: ScopedTask<'env, R> + 'env,
    {
        let name = name.into();
        self.push_task_dyn(name, Box::pin(task))
    }

    /// Spawns a task that yields once then completes.
    pub fn spawn_yield_now(&mut self, name: impl Into<String>) -> TaskId {
        self.spawn_named(name, YieldNowTask::new())
    }

    /// Hard stop immediately with reason.
    pub fn cancel_now_with_reason(&mut self, id: TaskId, reason: CancelReason) {
        if id >= self.tasks.len() || self.tasks[id].state != TaskState::Alive {
            return;
        }
        self.set_cancel_reason(id, reason);

        if let Some(t) = self.tracer.as_mut() {
            t.on_cancel_hard_stop(id, reason);
        }

        self.cancel_flags[id].store(true, Ordering::SeqCst);
        self.cancel_deadline[id] = None;
        self.cancel_time_deadline[id] = None;

        if let Some(ab) = self.abort_reason.get(id).and_then(|x| x.as_ref()) {
            let cur = ab.load(Ordering::Relaxed);
            if cur != ABORT_PANICKED && cur == 0 {
                ab.store(reason.as_u8(), Ordering::SeqCst);
            }
        }

        self.finish_task(id, TaskState::Cancelled);
    }

    pub fn cancel_now(&mut self, id: TaskId) {
        self.cancel_now_with_reason(id, CancelReason::UserRequest);
    }

    #[inline]
    fn schedule_cancel_deadlines(&mut self, id: TaskId, budgets: CancelBudgets) {
        // tick deadline
        if budgets.tick_budget > 0 {
            let dl = self.tick_count.saturating_add(budgets.tick_budget);
            self.cancel_deadline[id] = Some(dl);
            self.cancel_heap.push(Reverse((dl, id)));
        } else {
            self.cancel_deadline[id] = None;
        }

        // time deadline
        if let Some(dur) = budgets.time_budget {
            let dl = Instant::now() + dur;
            self.cancel_time_deadline[id] = Some(dl);
            self.cancel_time_heap.push(Reverse((dl, id)));
        } else {
            self.cancel_time_deadline[id] = None;
        }
    }

    /// Best-effort cancel with explicit escalation + reason.
    pub fn request_cancel_escalate_with_reason(
        &mut self,
        id: TaskId,
        esc: CancelEscalation,
        reason: CancelReason,
    ) {
        if id >= self.cancel_flags.len() || self.tasks[id].state != TaskState::Alive {
            return;
        }

        self.set_cancel_reason(id, reason);

        // flag + wake
        self.cancel_flags[id].store(true, Ordering::SeqCst);
        self.wake_tx.send(WakeMsg {
            from: EXTERNAL_WAKE,
            to: id,
        });

        let budgets = resolve_cancel_budgets(self.cancel_budget_default, self.cancel_time_default, esc);

        if let Some(tr) = self.tracer.as_mut() {
            tr.on_cancel_request(
                id,
                reason,
                if budgets.tick_budget > 0 {
                    Some(budgets.tick_budget)
                } else {
                    None
                },
                budgets.time_budget.filter(|d| *d != Duration::ZERO),
            );
        }

        // immediate hard-stop
        if budgets.immediate {
            self.cancel_now_with_reason(id, reason);
            return;
        }

        self.schedule_cancel_deadlines(id, budgets);
    }

    /// Default escalation cancel (uses scope defaults) with UserRequest.
    pub fn request_cancel_escalate(&mut self, id: TaskId) {
        self.request_cancel_escalate_with_reason(
            id,
            CancelEscalation::DEFAULT,
            CancelReason::UserRequest,
        );
    }

    // ---------------- joinable output ----------------

    pub fn spawn_with_result<T, Out, F>(
        &mut self,
        name: impl Into<String>,
        task: T,
        finish: F,
    ) -> JoinHandle<Out>
    where
        T: ScopedTask<'env, R> + 'env,
        Out: Send + 'env,
        F: FnOnce(Pin<&mut T>, &mut R) -> Out + 'env,
    {
        let (tx, rx) = mpsc::channel::<Result<Out, JoinError>>();
        let abort_reason = Arc::new(AtomicU8::new(0));

        struct OutputTask<'env, R, T, Out, F>
        where
            T: ScopedTask<'env, R> + 'env,
            Out: Send + 'env,
            F: FnOnce(Pin<&mut T>, &mut R) -> Out + 'env,
        {
            inner: Pin<Box<T>>,
            finish: Option<F>,
            tx: Option<mpsc::Sender<Result<Out, JoinError>>>,
            sent: bool,
            abort_reason: Arc<AtomicU8>,
            _p: std::marker::PhantomData<&'env mut R>,
        }

        impl<'env, R, T, Out, F> Unpin for OutputTask<'env, R, T, Out, F>
        where
            T: ScopedTask<'env, R> + 'env,
            Out: Send + 'env,
            F: FnOnce(Pin<&mut T>, &mut R) -> Out + 'env,
        {
        }

        impl<'env, R, T, Out, F> Drop for OutputTask<'env, R, T, Out, F>
        where
            T: ScopedTask<'env, R> + 'env,
            Out: Send + 'env,
            F: FnOnce(Pin<&mut T>, &mut R) -> Out + 'env,
        {
            fn drop(&mut self) {
                if self.sent {
                    return;
                }
                if let Some(tx) = self.tx.take() {
                    let code = self.abort_reason.load(Ordering::Relaxed);
                    let msg = if code == ABORT_PANICKED {
                        Err(JoinError::Panicked)
                    } else if code != 0 {
                        Err(JoinError::Cancelled(CancelReason::from_u8(code)))
                    } else {
                        Err(JoinError::Cancelled(CancelReason::Unknown))
                    };
                    let _ = tx.send(msg);
                }
            }
        }

        impl<'env, R, T, Out, F> ScopedTask<'env, R> for OutputTask<'env, R, T, Out, F>
        where
            T: ScopedTask<'env, R> + 'env,
            Out: Send + 'env,
            F: FnOnce(Pin<&mut T>, &mut R) -> Out + 'env,
        {
            fn poll(mut self: Pin<&mut Self>, cx: &mut Cx<'_, 'env, R>) -> TaskPoll {
                let this = self.as_mut().get_mut();

                // Cancellation dominates join output.
                if cx.is_cancelled() {
                    if !this.sent {
                        if let Some(tx) = this.tx.take() {
                            let code = this.abort_reason.load(Ordering::Relaxed);
                            let reason =
                                if code == 0 { CancelReason::Unknown } else { CancelReason::from_u8(code) };
                            let _ = tx.send(Err(JoinError::Cancelled(reason)));
                        }
                        this.sent = true;
                    }
                    return TaskPoll::Ready;
                }

                match this.inner.as_mut().poll(cx) {
                    TaskPoll::Pending => TaskPoll::Pending,
                    TaskPoll::Ready => {
                        if let (Some(f), Some(tx)) = (this.finish.take(), this.tx.take()) {
                            let out = f(this.inner.as_mut(), cx.resources);
                            let _ = tx.send(Ok(out));
                        }
                        this.sent = true;
                        TaskPoll::Ready
                    }
                }
            }
        }

        let wrapper = OutputTask {
            inner: Box::pin(task),
            finish: Some(finish),
            tx: Some(tx),
            sent: false,
            abort_reason: abort_reason.clone(),
            _p: std::marker::PhantomData,
        };

        let id = self.spawn_named(name, wrapper);
        self.abort_reason[id] = Some(abort_reason);

        JoinHandle { task: id, rx }
    }

    // ---------------- join combinator spawns ----------------

    pub fn spawn_map_join<T, Out, F>(
        &mut self,
        name: impl Into<String>,
        join: JoinHandle<T>,
        f: F,
    ) -> JoinHandle<Out>
    where
        T: Send + 'env,
        Out: Send + 'env,
        F: for<'p> FnOnce(Result<T, JoinError>, &mut Cx<'p, 'env, R>) -> Out + 'env,
    {
        let (tx, rx) = mpsc::channel::<Result<Out, JoinError>>();
        let producer = join.task;

        let task = MapJoinTask::<T, Out, F> {
            producer,
            join: Some(join),
            tx: Some(tx),
            f: Some(f),
            subscribed: false,
        };

        let id = self.spawn_named(name, task);
        JoinHandle { task: id, rx }
    }

    pub fn spawn_join_all<T>(
        &mut self,
        name: impl Into<String>,
        joins: Vec<JoinHandle<T>>,
    ) -> JoinHandle<Vec<Result<T, JoinError>>>
    where
        T: Send + 'env,
    {
        // IMPORTANT: this is already "the join handle message type":
        //   Result< (your output), JoinError >
        let (tx, rx) = mpsc::channel::<Result<Vec<Result<T, JoinError>>, JoinError>>();

        let producers = joins.iter().map(|j| j.task).collect::<Vec<_>>();
        let n = joins.len();

        let task = JoinAllTask::<T> {
            producers,
            joins,
            results: (0..n).map(|_| None).collect(),
            subscribed: vec![false; n],
            tx: Some(tx),
        };

        let id = self.spawn_named(name, task);

        // No extra task. If JoinAllTask panics/cancels, joining this handle returns Err(JoinError).
        JoinHandle { task: id, rx }
    }


    pub fn spawn_select2<A, B>(
        &mut self,
        name: impl Into<String>,
        a: JoinHandle<A>,
        b: JoinHandle<B>,
    ) -> JoinHandle<Either<Result<A, JoinError>, Result<B, JoinError>>>
    where
        A: Send + 'env,
        B: Send + 'env,
    {
        let (tx, rx) = mpsc::channel::<
            Result<Either<Result<A, JoinError>, Result<B, JoinError>>, JoinError>,
        >();

        let task = Select2Task::<A, B> {
            prod_a: a.task,
            prod_b: b.task,
            a: Some(a),
            b: Some(b),
            sub_a: false,
            sub_b: false,
            flip: false,
            tx: Some(tx),
        };

        let id = self.spawn_named(name, task);

        self.spawn_map_join("select2/unwrap", JoinHandle { task: id, rx }, |r, _cx| match r {
            Ok(v) => v,
            Err(e) => Either::Left(Err(e)),
        })
    }

    pub fn spawn_select_n<T>(
        &mut self,
        name: impl Into<String>,
        joins: Vec<JoinHandle<T>>,
    ) -> JoinHandle<(usize, Result<T, JoinError>)>
    where
        T: Send + 'env,
    {
        let (tx, rx) = mpsc::channel::<Result<(usize, Result<T, JoinError>), JoinError>>();
        let producers = joins.iter().map(|j| j.task).collect::<Vec<_>>();
        let n = joins.len();

        let task = SelectNTask::<T> {
            producers,
            joins,
            subscribed: vec![false; n],
            rr: 0,
            tx: Some(tx),
        };

        let id = self.spawn_named(name, task);

        self.spawn_map_join("select_n/unwrap", JoinHandle { task: id, rx }, |r, _cx| match r {
            Ok(v) => v,
            Err(e) => (usize::MAX, Err(e)),
        })
    }
    
    pub fn spawn_join_all_ok<T, E>(
        &mut self,
        name: impl Into<String>,
        joins: Vec<JoinHandle<Result<T, E>>>,
    ) -> JoinHandle<Result<Vec<T>, JoinAllOkError<E>>>
    where
        T: Send + 'env,
        E: Send + 'env,
    {
        let (tx, rx) =
            mpsc::channel::<Result<Result<Vec<T>, JoinAllOkError<E>>, JoinError>>();

        let producers = joins.iter().map(|j| j.task).collect::<Vec<_>>();
        let n = joins.len();

        let task = JoinAllOkTask::<T, E> {
            producers,
            joins,
            subscribed: vec![false; n],
            vals: (0..n).map(|_| None).collect(),
            tx: Some(tx),
        };

        let id = self.spawn_named(name, task);

        self.spawn_map_join("join_all_ok/unwrap", JoinHandle { task: id, rx }, |r, _cx| {
            match r {
                Ok(v) => v, // v: Result<Vec<T>, JoinAllOkError<E>>
                Err(e) => Err(JoinAllOkError::JoinAllFailed(e)),
            }
        })
    }

    // ---------------- scheduler driving ----------------

    pub fn tick_n(&mut self, n: usize) -> (usize, TickResult) {
        let mut progressed = 0usize;
        for _ in 0..n {
            match self.tick() {
                TickResult::Progress => progressed += 1,
                TickResult::Idle => return (progressed, TickResult::Idle),
                TickResult::Done => return (progressed, TickResult::Done),
            }
        }
        (
            progressed,
            if self.active == 0 {
                TickResult::Done
            } else {
                TickResult::Progress
            },
        )
    }

    pub fn run_idle(&mut self) -> TickResult {
        loop {
            match self.tick() {
                TickResult::Progress => continue,
                TickResult::Idle => return TickResult::Idle,
                TickResult::Done => return TickResult::Done,
            }
        }
    }

    pub fn run(&mut self) {
        while self.active > 0 {
            match self.tick() {
                TickResult::Progress => {}
                TickResult::Done => break,
                TickResult::Idle => {
                    // WASM/local: don’t block; return to outer event loop.
                    if !self.wake_rx.supports_blocking() {
                        return;
                    }

                    if let Some(wait) = self.time_to_next_deadline() {
                        match self.wake_rx.recv_timeout(wait) {
                            WakeRecvTimeout::Msg(m) => self.on_wake_msg(m),
                            WakeRecvTimeout::Timeout => {}
                            WakeRecvTimeout::Disconnected => break,
                        }
                    } else {
                        match self.wake_rx.recv() {
                            Some(m) => self.on_wake_msg(m),
                            None => break,
                        }
                    }
                }
            }
        }
    }

    pub fn run_until(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;

        while self.active > 0 {
            match self.tick() {
                TickResult::Progress => continue,
                TickResult::Done => return true,
                TickResult::Idle => {
                    if Instant::now() >= deadline {
                        return false;
                    }
                    let remain = deadline.saturating_duration_since(Instant::now());

                    // also wake on next timer/cancel deadline
                    let wait = match self.time_to_next_deadline() {
                        Some(t) => std::cmp::min(t, remain),
                        None => remain,
                    };

                    match self.wake_rx.recv_timeout(wait) {
                        WakeRecvTimeout::Msg(m) => self.on_wake_msg(m),
                        WakeRecvTimeout::Timeout => {}
                        WakeRecvTimeout::Disconnected => return false,
                    }
                }
            }
        }

        true
    }

    pub fn tick(&mut self) -> TickResult {
        if self.active == 0 {
            return TickResult::Done;
        }

        self.tick_count = self.tick_count.saturating_add(1);

        self.fire_expired_cancels();
        self.fire_expired_cancel_times();
        self.fire_expired_timers();

        self.drain_spawns();
        self.drain_wakes();

        let Some(id) = self.ready.pop_front() else {
            return TickResult::Idle;
        };

        if id >= self.queued.len() {
            return TickResult::Progress;
        }
        self.queued[id] = false;

        if self.tasks[id].state != TaskState::Alive {
            return TickResult::Progress;
        }

        self.poll_one(id);
        TickResult::Progress
    }

    // ---------------- internals ----------------

    #[inline]
    fn enqueue_ready(&mut self, id: TaskId) {
        if id >= self.queued.len()
            || self.tasks[id].state != TaskState::Alive
            || self.queued[id]
        {
            return;
        }
        self.queued[id] = true;
        self.ready.push_back(id);
    }

    fn on_wake_msg(&mut self, msg: WakeMsg) {
        if let Some(t) = self.tracer.as_mut() {
            t.on_wake(msg.from, msg.to);
        }
        self.enqueue_ready(msg.to);
    }

    fn drain_wakes(&mut self) {
        while let Some(msg) = self.wake_rx.try_recv() {
            self.on_wake_msg(msg);
        }
    }

    fn drain_spawns(&mut self) {
        while let Some(req) = self.spawns.pop_front() {
            if let Some(t) = self.tracer.as_mut() {
                t.on_defer_spawn(req.parent, &req.name);
            }
            let name = req.name;
            self.push_task_dyn(name, req.task);
        }
    }

    fn finish_task(&mut self, id: TaskId, state: TaskState) {
        let name = self.tasks[id].name.clone();

        self.tasks[id].state = state;
        self.tasks[id].task = None;

        self.sleeping_until[id] = None;
        self.cancel_deadline[id] = None;
        self.cancel_time_deadline[id] = None;
        self.cancel_reason[id] = CancelReason::None.as_u8();

        self.active = self.active.saturating_sub(1);
        self.queued[id] = false;

        if let Some(t) = self.tracer.as_mut() {
            t.on_finish(id, &name, state);
        }

        let waiters = std::mem::take(&mut self.finish_waiters[id]);
        for waiter in waiters {
            self.wake_tx.send(WakeMsg { from: id, to: waiter });
        }
    }

    // ---- cancel bookkeeping ----

    #[inline]
    fn should_replace_reason(cur: CancelReason, new: CancelReason) -> bool {
        // Conservative policy: don’t overwrite a “real” reason except with timeouts.
        match cur {
            CancelReason::None | CancelReason::Unknown => true,
            _ => matches!(new, CancelReason::TimeoutTicks | CancelReason::TimeoutTime),
        }
    }

    #[inline]
    fn set_cancel_reason(&mut self, id: TaskId, new_reason: CancelReason) {
        if id >= self.cancel_reason.len() {
            return;
        }

        let cur = CancelReason::from_u8(self.cancel_reason[id]);
        if !Self::should_replace_reason(cur, new_reason) {
            return;
        }

        self.cancel_reason[id] = new_reason.as_u8();

        // Joinable abort code (don’t override panicked).
        if let Some(ab) = self.abort_reason.get(id).and_then(|x| x.as_ref()) {
            let mut cur = ab.load(Ordering::Relaxed);
            loop {
                if cur == ABORT_PANICKED {
                    break;
                }
                if cur != 0 && !Self::should_replace_reason(CancelReason::from_u8(cur), new_reason)
                {
                    break;
                }
                match ab.compare_exchange_weak(
                    cur,
                    new_reason.as_u8(),
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(v) => cur = v,
                }
            }
        }
    }

    fn fire_expired_cancels(&mut self) {
        loop {
            let Some(Reverse((dl, id))) = self.cancel_heap.peek().cloned() else { break; };
            if dl > self.tick_count {
                break;
            }
            self.cancel_heap.pop();

            if id >= self.tasks.len() || self.tasks[id].state != TaskState::Alive {
                continue;
            }
            if self.cancel_deadline.get(id).copied().flatten() != Some(dl) {
                continue; // stale
            }
            if !self.cancel_flags[id].load(Ordering::Relaxed) {
                continue; // no longer cancelled
            }

            self.cancel_now_with_reason(id, CancelReason::TimeoutTicks);
        }
    }

    fn fire_expired_cancel_times(&mut self) {
        let now = Instant::now();
        loop {
            let Some(Reverse((dl, id))) = self.cancel_time_heap.peek().cloned() else { break; };
            if dl > now {
                break;
            }
            self.cancel_time_heap.pop();

            if id >= self.tasks.len() || self.tasks[id].state != TaskState::Alive {
                continue;
            }
            if self.cancel_time_deadline.get(id).copied().flatten() != Some(dl) {
                continue; // stale
            }
            if !self.cancel_flags[id].load(Ordering::Relaxed) {
                continue;
            }

            self.cancel_now_with_reason(id, CancelReason::TimeoutTime);
        }
    }

    // ---- timer wheel ----

    fn fire_expired_timers(&mut self) {
        let now = Instant::now();
        while let Some(Reverse((dl, task))) = self.sleep_heap.peek().cloned() {
            if dl > now {
                break;
            }
            self.sleep_heap.pop();

            if task >= self.sleeping_until.len() {
                continue;
            }
            if self.sleeping_until[task] == Some(dl) && self.tasks[task].state == TaskState::Alive {
                self.sleeping_until[task] = None;
                self.wake_tx.send(WakeMsg {
                    from: EXTERNAL_WAKE,
                    to: task,
                });
            }
        }
    }

    fn time_to_next_deadline(&self) -> Option<Duration> {
        let now = Instant::now();

        let next_sleep = self.sleep_heap.peek().cloned().map(|Reverse((dl, _))| dl);
        let next_cancel = self
            .cancel_time_heap
            .peek()
            .cloned()
            .map(|Reverse((dl, _))| dl);

        let next = match (next_sleep, next_cancel) {
            (None, None) => return None,
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (Some(a), Some(b)) => if a <= b { a } else { b },
        };

        Some(next.saturating_duration_since(now))
    }

    // ---- poll_one (safe: take task out, poll, put back) ----
    fn poll_one(&mut self, id: TaskId) {
        self.poll_seq += 1;
        let seq = self.poll_seq;

        let name = match self.tasks.get(id) {
            Some(e) => e.name.clone(),
            None => return,
        };

        // Take task out so we can borrow other fields mutably for hooks.
        let mut task = match self.tasks[id].task.take() {
            Some(t) => t,
            None => return,
        };

        if let Some(t) = self.tracer.as_mut() {
            t.on_poll_seq(seq, id, &name);
            t.on_poll_begin(id, &name);
        }

        let wake_tx = self.wake_tx.clone();
        let cancel_flag = self.cancel_flags[id].clone();

        let poll_res = {
            // Split borrows (everything disjoint).
            let tasks_ref: &[TaskEntry<'env, R>] = &self.tasks;

            let finish_waiters = &mut self.finish_waiters;
            let spawns = &mut self.spawns;
            let io_read_waiters = &mut self.io_read_waiters;

            let sleeping_until = &mut self.sleeping_until;
            let sleep_heap = &mut self.sleep_heap;

            let cancel_flags = &self.cancel_flags;
            let cancel_reason = &mut self.cancel_reason;
            let abort_reason = &self.abort_reason;

            let tick_count = self.tick_count;
            let cancel_budget_default = self.cancel_budget_default;
            let cancel_time_default = self.cancel_time_default;

            let cancel_deadline = &mut self.cancel_deadline;
            let cancel_heap = &mut self.cancel_heap;

            let cancel_time_deadline = &mut self.cancel_time_deadline;
            let cancel_time_heap = &mut self.cancel_time_heap;

            // SAFETY: `tracer_ptr` comes from a valid &mut self.tracer as NonNull<dyn Tracer>,
            // and is only accessed once per poll loop in a serialized manner (no aliasing).
            let tracer_ptr: Option<NonNull<dyn Tracer>> = self.tracer.as_deref_mut().map(NonNull::from);

            // Hooks
            let mut subscribe_finish = |waiter: TaskId, target: TaskId| {
                if let Some(mut tr) = tracer_ptr {
                    unsafe{
                        tr.as_mut().on_subscribe_finish(waiter, target);
                    }
                }
                if let Some(v) = finish_waiters.get_mut(target) {
                    v.push(waiter);
                }
            };

            let mut defer_spawn =
                |parent: TaskId, name: String, task: Pin<Box<dyn ScopedTask<'env, R> + 'env>>| {
                    spawns.push_back(SpawnReq { parent, name, task });
                };

            let mut query_state = |qid: TaskId| {
                tasks_ref
                    .get(qid)
                    .map(|t| t.state)
                    .unwrap_or(TaskState::Completed)
            };

            let mut subscribe_io_readable = |task_id: TaskId, token: usize| {
                if token >= io_read_waiters.len() {
                    io_read_waiters.resize_with(token + 1, Vec::new);
                }
                io_read_waiters[token].push(task_id);

                if let Some(mut tr) = tracer_ptr {
                    unsafe {
                        tr.as_mut().on_io_wait_readable(task_id, token);
                    }
                }
            };

            let mut subscribe_sleep_until = |task_id: TaskId, deadline: Instant| {
                if task_id >= sleeping_until.len() {
                    return;
                }
                sleeping_until[task_id] = Some(deadline);
                sleep_heap.push(Reverse((deadline, task_id)));

                if let Some(mut tr) = tracer_ptr {
                    let micros = deadline
                        .saturating_duration_since(Instant::now())
                        .as_micros();
                    unsafe {
                        tr.as_mut().on_sleep_until(task_id, micros);
                    }
                }
            };

            let mut request_cancel_escalate_reason =
                |target: TaskId, esc: CancelEscalation, reason: CancelReason| {
                    hook_request_cancel_escalate::<R>(
                        target,
                        esc,
                        reason,
                        tasks_ref,
                        cancel_flags,
                        cancel_reason,
                        abort_reason,
                        tick_count,
                        cancel_budget_default,
                        cancel_time_default,
                        cancel_deadline,
                        cancel_heap,
                        cancel_time_deadline,
                        cancel_time_heap,
                        tracer_ptr,     // ✅ correct type now
                        &wake_tx,
                    );
                };


            let mut cx = Cx {
                resources: &mut *self.resources,
                id,
                wake_tx: wake_tx.clone(),
                cancel_flag,
                subscribe_finish: &mut subscribe_finish,
                defer_spawn: &mut defer_spawn,
                query_state: &mut query_state,
                subscribe_io_readable: &mut subscribe_io_readable,
                subscribe_sleep_until: &mut subscribe_sleep_until,
                request_cancel_escalate_reason: &mut request_cancel_escalate_reason,
            };

            catch_unwind(AssertUnwindSafe(|| task.as_mut().poll(&mut cx)))
        };

        if let Some(t) = self.tracer.as_mut() {
            t.on_poll_end(id, &name);
        }

        match poll_res {
            Ok(TaskPoll::Pending) => {
                // Put task back (still alive).
                self.tasks[id].task = Some(task);
            }
            Ok(TaskPoll::Ready) => {
                self.finish_task(id, TaskState::Completed);
            }
            Err(_) => {
                if let Some(ab) = self.abort_reason.get(id).and_then(|x| x.as_ref()) {
                    ab.store(ABORT_PANICKED, Ordering::SeqCst);
                }
                self.finish_task(id, TaskState::Panicked);
            }
        }
    }
}

// ----------------------------- poll hook helper -----------------------------
#[inline]
fn hook_request_cancel_escalate<'env, R>(
    target: TaskId,
    esc: CancelEscalation,
    reason: CancelReason,
    tasks: &[TaskEntry<'env, R>],
    cancel_flags: &[Arc<AtomicBool>],
    cancel_reason: &mut [u8],
    abort_reason: &[Option<Arc<AtomicU8>>],
    tick_count: u64,
    cancel_budget_default: u64,
    cancel_time_default: Option<Duration>,
    cancel_deadline: &mut [Option<u64>],
    cancel_heap: &mut BinaryHeap<Reverse<(u64, TaskId)>>,
    cancel_time_deadline: &mut [Option<Instant>],
    cancel_time_heap: &mut BinaryHeap<Reverse<(Instant, TaskId)>>,
    // tracer: Option<&mut dyn Tracer>,
    tracer: Option<NonNull<dyn Tracer>>,
    wake_tx: &WakeTx,
) {
    if target >= tasks.len()
        || target >= cancel_flags.len()
        || tasks[target].state != TaskState::Alive
    {
        return;
    }

    // reason write (simple “first wins unless timeout” policy)
    {
        let cur = CancelReason::from_u8(cancel_reason[target]);
        if Scope::<R>::should_replace_reason(cur, reason) {
            cancel_reason[target] = reason.as_u8();
        }
    }

    // joinable abort code (don’t override panicked)
    if let Some(ab) = abort_reason.get(target).and_then(|x| x.as_ref()) {
        let cur = ab.load(Ordering::Relaxed);
        if cur != ABORT_PANICKED && cur == 0 {
            ab.store(reason.as_u8(), Ordering::SeqCst);
        }
    }

    // set flag + wake
    cancel_flags[target].store(true, Ordering::SeqCst);
    wake_tx.send(WakeMsg {
        from: EXTERNAL_WAKE,
        to: target,
    });

    let budgets = resolve_cancel_budgets(cancel_budget_default, cancel_time_default, esc);

    if let Some(mut tr) = tracer {
        unsafe {
            tr.as_mut().on_cancel_request(
                target,
                reason,
                if budgets.tick_budget > 0 {
                    Some(budgets.tick_budget)
                } else {
                    None
                },
                budgets.time_budget.filter(|d| *d != Duration::ZERO),
            );
        }
    }

    // In-hook "Immediate": schedule time deadline at now (can’t call finish_task here).
    if budgets.immediate {
        let dl = Instant::now();
        cancel_time_deadline[target] = Some(dl);
        cancel_time_heap.push(Reverse((dl, target)));
        return;
    }

    // tick deadline
    if budgets.tick_budget > 0 {
        let dl = tick_count.saturating_add(budgets.tick_budget);
        cancel_deadline[target] = Some(dl);
        cancel_heap.push(Reverse((dl, target)));
    } else {
        cancel_deadline[target] = None;
    }

    // time deadline
    if let Some(dur) = budgets.time_budget {
        let dl = Instant::now() + dur;
        cancel_time_deadline[target] = Some(dl);
        cancel_time_heap.push(Reverse((dl, target)));
    } else {
        cancel_time_deadline[target] = None;
    }
}