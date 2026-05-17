//! Tracing domain contracts.
//!
//! Canonical type and trait definitions for deterministic event tracing in the
//! Laplace simulation engine. Concrete backend implementations live in `laplace-core`;
//! this module defines the shared interface consumed by both crates.
//!
//! # Contents
//!
//! - [`types`]: `MAX_THREADS`, `ThreadId`, `LamportTimestamp`, `EventMetadata`,
//!   `FenceType`, `MemoryOperation`, `SyncEvent`, `ClockEvent`, `SimulationEvent`
//! - [`traits`]: `TracingError`, `TracerBackend`

pub mod traits;
pub mod types;

pub use traits::{TracerBackend, TracingError};
pub use types::{
    ClockEvent, EventMetadata, FenceType, LamportTimestamp, MemoryOperation, SimulationEvent,
    SyncEvent, ThreadId, MAX_THREADS,
};
