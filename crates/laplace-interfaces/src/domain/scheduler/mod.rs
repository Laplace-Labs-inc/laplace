//! Scheduler domain contracts.
//!
//! Canonical type and trait definitions for the Laplace thread scheduling subsystem.
//! Concrete backend implementations live in `laplace-core`; this module defines
//! the shared interface consumed by both the core and twin crates.
//!
//! # Contents
//!
//! - [`types`]: `ThreadId`, `TaskId`, `ThreadState`, `SchedulingStrategy`, `SchedulerError`
//! - [`traits`]: `EventId`, `SchedulerBackend`

pub mod traits;
pub mod types;

pub use traits::{EventId, SchedulerBackend};
pub use types::{SchedulerError, SchedulingStrategy, TaskId, ThreadId, ThreadState};
