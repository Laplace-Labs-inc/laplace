//! # Laplace Probe Adapter
//!
//! Bridges the kernel observation layer (`laplace-probe-common`) to the DPOR
//! verification engine (`laplace-axiom`) by maintaining compact thread and
//! resource index mappings.
//!
//! ## Architecture
//!
//! ```text
//! ┌───────────────────┐        ┌──────────────────────┐
//! │  Kernel Events    │        │  DPOR Scheduler      │
//! │  (RawProbeEvent)  │──────► │  (laplace-axiom)     │
//! └─────────┬─────────┘        └──────────────────────┘
//!           │  ProbeEventDecoder              ▲
//!           ▼                                 │
//! ┌───────────────────┐        ┌──────────────┴───────┐
//! │  DecodedProbeEvent│──────► │  AxiomStepBuilder    │
//! └───────────────────┘        │  (this crate)        │
//!                              └──────────────────────┘
//! ```
//!
//! ## Key Types
//!
//! - [`AxiomStepBuilder`]: Translates decoded events into DPOR-consumable steps.
//! - [`ThreadRegistry`]: Maps kernel TIDs to compact DPOR thread indices.
//! - [`ResourceRegistry`]: Maps kernel resource IDs to compact DPOR resource indices.

// Re-export adapter types from laplace-probe-common
pub use laplace_probe_common::{
    AxiomEvent, AxiomOp, AxiomResourceId, AxiomStep, AxiomStepBuilder, AxiomThreadId,
    ResourceRegistry, ThreadRegistry, MAX_AXIOM_THREADS,
};

// Re-export laplace-interfaces boundary types for callers
pub use laplace_interfaces::domain::resource::types::{ResourceId, ThreadId};

/// Converts a compact DPOR [`AxiomThreadId`] index to the interfaces [`ThreadId`] wrapper.
///
/// Use this at the boundary between the probe adapter and the DPOR scheduler:
/// ```ignore
/// let thread_id = laplace_probe_adapter::to_thread_id(axiom_step.thread);
/// ```
#[inline(always)]
pub fn to_thread_id(axiom_id: AxiomThreadId) -> ThreadId {
    ThreadId::new(axiom_id)
}

/// Converts a compact DPOR [`AxiomResourceId`] index to the interfaces [`ResourceId`] wrapper.
///
/// Use this at the boundary between the probe adapter and the DPOR scheduler:
/// ```ignore
/// let resource_id = laplace_probe_adapter::to_resource_id(axiom_step.resource);
/// ```
#[inline(always)]
pub fn to_resource_id(axiom_id: AxiomResourceId) -> ResourceId {
    ResourceId::new(axiom_id)
}
