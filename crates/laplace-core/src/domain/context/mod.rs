//! # Context Module
//!
//! Core context infrastructure for the entire Laplace stack.
//! Implements the Deterministic Context principle: all async operations
//! and domain logic receive explicit context parameters.
//!
//! # Architecture
//!
//! The context module is organized into focused submodules:
//!
//! - `primitives`: Foundational enumeration types (PriorityLevel, TenantTier)
//! - `sovereign`: The canonical SovereignContext struct and operations
//! - `builder`: Fluent API for ergonomic context construction
//!
//! All public types are re-exported at the module level for convenient access.
//!
//! # Single Source of Truth (SSOT)
//!
//! The Rust structures in this module define the authoritative context format.
//! All other representations (Protobuf, FFI, TypeScript) are derived from
//! these definitions.
//!
//! # Examples
//!
//! Creating contexts using factory methods:
//!
//! ```ignore
//! use laplace_core::domain::context::{SovereignContext, TenantTier};
//!
//! // Standard context
//! let ctx = SovereignContext::new(
//!     "req-123".to_string(),
//!     "tenant-acme".to_string(),
//!     "trace-xyz".to_string(),
//! );
//!
//! // Turbo-mode context
//! let turbo_ctx = SovereignContext::new_turbo(
//!     "req-456".to_string(),
//!     "tenant-premium".to_string(),
//!     "trace-fast".to_string(),
//! ).with_tier(TenantTier::Enterprise);
//! ```
//!
//! Creating contexts using the builder API:
//!
//! ```ignore
//! use laplace_core::domain::context::{ContextBuilder, TenantTier, PriorityLevel};
//!
//! let ctx = ContextBuilder::new("req-789")
//!     .tenant("tenant-acme")
//!     .trace("trace-xyz")
//!     .tier(TenantTier::Enterprise)
//!     .priority(PriorityLevel::Critical)
//!     .turbo(true)
//!     .build();
//! ```

mod builder;

#[cfg(kani)]
pub mod proofs;

pub use builder::ContextBuilder;

// Re-export interface types for proof use
pub use crate::domain::TenantTier;
pub use laplace_interfaces::domain::{PriorityLevel, SovereignContext, NO_TURBO_SLOT};
