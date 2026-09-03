//! One switch point for synchronization primitives.
//!
//! Normal builds use `std`. Under `--cfg loom` (the `loom` feature) we use
//! `loom`'s instrumented equivalents so the model checker can explore every
//! interleaving of the cross-thread wake / cancel handoff. Keeping the alias in
//! one place is what makes the whole runtime loom-testable without sprinkling
//! `#[cfg]` through the scheduler.

#[cfg(loom)]
pub use loom::sync::{Arc, Condvar, Mutex};
#[cfg(loom)]
pub use loom::sync::atomic::{AtomicBool, AtomicU8, Ordering};

#[cfg(not(loom))]
pub use std::sync::{Arc, Condvar, Mutex};
#[cfg(not(loom))]
pub use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
