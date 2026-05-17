//! Scheduler backend trait — re-exported from `laplace-interfaces`
//!
//! The canonical `SchedulerBackend` trait and `EventId` type live in
//! `laplace_interfaces::domain::scheduler::traits`. This file re-exports them so
//! that code within `laplace-core` can continue to use
//! `crate::domain::scheduler::{SchedulerBackend, EventId}`.

pub use laplace_interfaces::domain::scheduler::traits::{EventId, SchedulerBackend};
