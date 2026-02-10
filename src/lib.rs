#[cfg(feature = "mio")]
pub mod mio;
pub mod io;
pub mod lease;
pub mod scope;
pub mod stream;
pub mod task;
pub mod trace;
pub mod ops;

#[macro_use]
pub mod macros;

pub mod select;
pub mod timer;

pub use io::{IoToken, IoWaitKind};
pub use lease::{Lease, OnDrop};
pub use scope::{
    // scoped, JoinError, JoinHandle, JoinPoll, Scope, TickResult,
    scoped, JoinHandle, Scope, TickResult,
};
pub use select::Either;
pub use timer::SleepTask;
pub use stream::{ScopedStream, StreamForEachTask};
pub use task::{Cx, FnTask, ScopedTask, TaskId, TaskPoll, TaskState, JoinError, JoinPoll};
pub use trace::{DotTracer, LogTracer, Tracer};
pub mod wake;
pub use wake::*;
// src/lib.rs (add)
pub mod any;
pub mod cancel;

pub use any::{AnyBox, TaggedAny};
pub use cancel::CancelToken;




pub mod cancel_policy;

pub use cancel_policy::{CancelEscalation, TickEscalation, TimeEscalation};

pub mod cancel_reason;
pub use cancel_reason::{CancelReason, ABORT_PANICKED};
