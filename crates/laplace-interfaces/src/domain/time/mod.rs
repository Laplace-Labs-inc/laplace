//! Time domain contracts.
//!
//! Canonical type and trait definitions for the Laplace virtual clock abstraction.
//! Concrete backend implementations (`ProductionBackend`, `VerificationBackend`) live
//! in `laplace-core`; this module defines the shared interface consumed by both crates.
//!
//! # Contents
//!
//! - [`types`]: `VirtualTimeNs`, `LamportClock`, `EventId`, `TimeMode`, `EventPayload`, `ScheduledEvent`
//! - [`traits`]: `ClockBackend`

pub mod traits;
pub mod types;

pub use traits::ClockBackend;
pub use types::{EventId, EventPayload, LamportClock, ScheduledEvent, TimeMode, VirtualTimeNs};
