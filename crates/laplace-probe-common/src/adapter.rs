// SPDX-License-Identifier: Apache-2.0
//! # Laplace Probe Adapter
//!
//! Boundary helpers for converting compact public probe identifiers into the
//! shared interface wrapper types.

// Re-export laplace-interfaces boundary types for callers
pub use laplace_interfaces::domain::resource::types::{ResourceId, ThreadId};

/// Converts a compact thread index to the interfaces [`ThreadId`] wrapper.
///
/// Use this at the boundary between probe decoding and a consumer-specific
/// scheduler or analysis engine:
/// ```ignore
/// let thread_id = laplace_probe_common::adapter::to_thread_id(step.thread);
/// ```
#[inline(always)]
pub fn to_thread_id(index: usize) -> ThreadId {
    ThreadId::new(index)
}

/// Converts a compact resource index to the interfaces [`ResourceId`] wrapper.
///
/// Use this at the boundary between probe decoding and a consumer-specific
/// scheduler or analysis engine:
/// ```ignore
/// let resource_id = laplace_probe_common::adapter::to_resource_id(step.resource);
/// ```
#[inline(always)]
pub fn to_resource_id(index: usize) -> ResourceId {
    ResourceId::new(index)
}
