//! Laplace Deadpool Audit — Formal concurrency verification of `deadpool` 0.10.
//!
//! # Surgical Patching Strategy
//!
//! This crate contains a **source-level copy** of `deadpool 0.10.0`'s managed pool
//! with a surgical patch applied to its synchronisation layer.
//!
//! ## What is patched
//!
//! | Original (`deadpool 0.10`)           | Patched (`feature = "axiom"`)         |
//! |--------------------------------------|---------------------------------------|
//! | `tokio::sync::Semaphore`             | `axiom_compat::AxiomSemaphore`        |
//! | `tokio::sync::TryAcquireError`       | `axiom_compat::AxiomTryAcquireError`  |
//!
//! `std::sync::Mutex` is intentionally **not** replaced: deadpool holds the mutex
//! only transiently within a single thread (brief critical section), so it never
//! contributes to inter-thread deadlock cycles.  The `tokio::sync::Semaphore` is
//! the sole inter-thread blocking primitive and is the root cause of all pool
//! slot contention.
//!
//! ## How the patch works
//!
//! When the `axiom` Cargo feature is enabled the `use` statement in
//! `managed/mod.rs` is replaced (via `#[cfg]` guards) from:
//!
//! ```text
//! use tokio::sync::{Semaphore, TryAcquireError};  // original
//! ```
//!
//! to:
//!
//! ```text
//! use crate::axiom_compat::{             // patched
//!     AxiomSemaphore        as Semaphore,
//!     AxiomTryAcquireError  as TryAcquireError,
//! };
//! ```
//!
//! `AxiomSemaphore` has an identical API to `tokio::sync::Semaphore` but each
//! `acquire()` / `add_permits()` call also writes an entry to a **thread-local
//! operation log** (`axiom_compat::op_log`).  The DPOR harness reads from this
//! log to obtain the `(Operation, ResourceId)` sequence for each virtual thread.
//!
//! ## Stress harness
//!
//! The `stress` module (enabled with the `stress` feature of `laplace-harness`)
//! runs the patched `ManagedPool` under a real Tokio runtime, driving tens of
//! thousands of `pool.get()` / `drop(conn)` cycles and checking the invariant
//! `available + in_use == max_size` after every operation.

#![forbid(unsafe_code)]
#![allow(missing_docs)]
#![allow(unused_results)]
#![allow(deprecated)]

pub mod axiom_compat;
pub mod managed;

pub use deadpool_runtime::{Runtime, SpawnBlockingError};

/// Pool status snapshot — mirrors `deadpool::Status`.
#[derive(Clone, Copy, Debug)]
pub struct Status {
    pub max_size: usize,
    pub size: usize,
    pub available: usize,
    pub waiting: usize,
}
