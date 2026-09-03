//! # scope
//!
//! A single-threaded, cooperative, **deterministic** structured-concurrency
//! runtime whose tasks may borrow non-`'static` data from the scope environment
//! — the thing real `async` Rust cannot safely do with spawned tasks.
//!
//! ## What makes it tick
//!
//! * **Scoped borrows.** A [`ScopedTask`] is polled `&mut self` and handed a
//!   [`Cx`] carrying `&mut R`, a live exclusive borrow of the environment.
//!   Safe because polling is cooperative and single-threaded.
//! * **A real waker substrate.** Waking rides [`WakeBus`] and mints genuine
//!   [`std::task::Waker`]s (via `waker_tables`), so external IO / timers / other
//!   threads can wake a scoped task by generational [`TaskId`].
//! * **Policy-driven scheduling.** The runnable set is ordered each round by a
//!   [`WakeOrdering`] policy. The default policy was chosen by running the
//!   candidates against deterministic workload suites, not by taste.
//! * **Structured `run()`.** Drives to completion — the scope boundary is a
//!   join point — and reports stalled tasks instead of hanging.
//!
//! `unsafe` is confined to [`wake`] (the audited `waker_tables` plumbing);
//! everything else is `#![deny(unsafe_code)]`.

#![deny(unsafe_code)]

pub mod sync;

pub mod cancel;
pub mod combinators;
pub mod ids;
pub mod mailbox;
pub mod policy;
pub mod scope;
pub mod select;
pub mod task;
pub mod timer;
pub mod trace;
pub mod wake;

pub use cancel::{CancelEscalation, CancelReason, TickEscalation, TimeEscalation};
pub use combinators::{JoinAllOkError, JoinHandle};
pub use ids::TaskId;
pub use mailbox::{Mailbox, MailboxSender};
pub use policy::{QoSClass, SchedulerConfig, WakeOrdering};
pub use scope::{run_scoped, scoped, RunReport, Scope, TickOutcome};
pub use select::Either;
pub use task::{Cx, FnTask, JoinError, JoinPoll, ScopedTask, TaskPoll, TaskState, YieldNowTask};
pub use timer::SleepTask;
pub use trace::{BufferTracer, CountingTracer, LogTracer, Tracer};
pub use wake::{SchedulerMsg, WakeBus, WakeReason};
#[cfg(not(loom))]
pub use wake::scheduler_waker;

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
mod readme_doctests {}
