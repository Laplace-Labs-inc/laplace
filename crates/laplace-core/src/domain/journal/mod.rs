// SPDX-License-Identifier: Apache-2.0
//! Journal Domain Layer
//!
//! Migrated from laplace-axiom to laplace-core (Phase R2: Brain Migration - Step 2).
//!
//! This module defines the transaction journaling system for comprehensive audit
//! trails across standard FFI and Turbo-accelerated execution paths. The journal
//! provides a unified interface for both kernel-side execution tracking and
//! observation by KNUL/Axiom components.
//!
//! # Architecture
//!
//! The journal module comprises two core components that work in tandem:
//!
//! - **LogStatus**: Enumeration representing the complete lifecycle of tenant
//!   operations from submission through terminal completion. Enables status
//!   checking, code conversion for protobuf, and classification of execution
//!   outcomes.
//!
//! - **TransactionLog**: Entity capturing immutable audit trail entries with
//!   comprehensive metadata including request tracing, tenant isolation,
//!   operation classification, duration metrics, and zero-copy acceleration
//!   tracking.
//!
//! # Zero-Copy Acceleration Tracking
//!
//! The transaction log intelligently tracks both execution models:
//!
//! Standard FFI Path: Uses Protobuf serialization with approximately 41.5µs
//! context synchronization overhead. Suitable for general-purpose operations
//! with moderate performance requirements.
//!
//! Turbo Acceleration: Uses shared memory zero-copy with target latency below
//! 500 nanoseconds. Requires slot allocation in the shared memory pool and is
//! only available to higher-tier customers.
//!
//! The `is_turbo` flag on TransactionLog disambiguates the execution path, while
//! optional `turbo_slot_index` and `turbo_memory_offset` fields provide allocation
//! metadata for performance analysis and debugging.
//!
//! # Spec Compliance
//!
//! - Sovereign-002: Transaction audit trail for execution tracking
//! - Spec-008: Status code mapping for SDK propagation
//!
//! # Example: Standard FFI Execution
//!
//! ```ignore
//! use laplace_core::domain::journal::{TransactionLog, LogStatus};
//!
//! let log = TransactionLog::new(
//!     "req-123".into(),
//!     "tenant-abc".into(),
//!     "execute_script".into(),
//!     LogStatus::Success,
//! )
//! .with_duration(42_000); // 42µs (typical Standard FFI)
//!
//! assert!(!log.is_turbo_execution());
//! assert_eq!(log.latency_category(), "medium");
//! ```
//!
//! # Example: Turbo Acceleration Execution
//!
//! ```ignore
//! use laplace_core::domain::journal::{TransactionLog, LogStatus};
//!
//! let log = TransactionLog::new_turbo(
//!     "req-456".into(),
//!     "tenant-xyz".into(),
//!     "execute_script".into(),
//!     LogStatus::Success,
//!     42,    // slot_index
//!     8192,  // memory_offset
//! )
//! .with_duration(450); // 450ns (Turbo target)
//!
//! assert!(log.is_turbo_execution());
//! assert_eq!(log.turbo_slot_info(), Some((42, 8192)));
//! assert_eq!(log.latency_category(), "sub-microsecond");
//! ```

pub mod ard;
pub mod model;
pub mod status;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Public Re-exports
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Transaction execution lifecycle state enumeration
pub use status::LogStatus;

/// Immutable audit trail entry for tenant operation execution
pub use model::TransactionLog;

/// ARD (Axiom Recorded Data) forensic format — ultra-deterministic 21-step replay
pub use ard::{
    ArdHeader, ArdReport, ForensicFrame, ForensicWindow, WINDOW_POST, WINDOW_PRE, WINDOW_TOTAL,
};

#[cfg(kani)]
mod proofs;
