//! # SCOPE
//! Deterministic execution boundaries and lifetime-orchestrated concurrency.
//! 
//! This crate provides primitives for managing scoped execution flows, 
//! ensuring that asynchronous tasks and memory references remain strictly 
//! confined to their intended lifetimes.
//!
//! ## Architectural Principles
//! * **Containment:** Guarantees that execution cannot outlive its defined scope.
//! * **Deterministic Cleanup:** Synchronous-style resource management for asynchronous flows.
//! * **Zero-Cost Abstraction:** Leverages Rust's borrow checker to enforce execution boundaries 
//!   at compile-time without runtime overhead.
//! * **Safety:** Eliminates "escaped reference" vulnerabilities in complex orchestrations.