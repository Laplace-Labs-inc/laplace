//! Pool Management Domain Layer
//!
//! Migrated from laplace-axiom to laplace-core (Phase R2: Brain Migration - Step 3).
//!
//! This module defines the complete decision logic and state representation for
//! resource pool management. It maintains independence from infrastructure concerns
//! (V8, Tokio, Sled) by containing only pure business logic and policy decisions.
//!
//! # Architecture
//!
//! The pool domain comprises two core components:
//!
//! **Policy Module**: Contains all decision-making functions that determine how
//! resources should be allocated, evicted, or preempted based on tenant tier and
//! resource constraints. The adapters layer implements these decisions.
//!
//! **State Module**: Defines the observable metrics and health status that represent
//! the instantaneous state of pool utilization. Enables comprehensive monitoring
//! and capacity planning through structured observations.
//!
//! # Dual Execution Paths
//!
//! The pool intelligently manages two distinct execution paths with separate capacity:
//!
//! **Turbo Path**: Zero-copy shared memory acceleration providing sub-500-nanosecond
//! context synchronization. Available exclusively to Turbo, Pro, and Enterprise tier
//! customers. Limited capacity requires sophisticated preemption and fallback logic.
//!
//! **Standard Path**: Protobuf FFI-based marshaling providing approximately 41.5
//! microsecond context synchronization. Available to all tiers. Larger capacity
//! accommodates the broader tenant base with acceptable latency characteristics.
//!
//! # Preemption Hierarchy
//!
//! When resources become scarce, the preemption hierarchy ensures that higher-tier
//! paying customers maintain quality of service:
//!
//! - Free tier tenants can preempt no one
//! - Standard tier can preempt Free tier
//! - Turbo tier can preempt Free and Standard
//! - Pro tier can preempt Free and Standard
//! - Enterprise tier can preempt all lower tiers
//!
//! This hierarchy enables economically-sustainable overcommit strategies where the
//! system can safely serve more tenants than peak capacity suggests possible.
//!
//! # Spec Compliance
//!
//! - Sovereign-001: Isolate pooling and lifecycle management
//! - Performance: Turbo vs Standard execution path separation
//! - Preemption: Tier-based resource reclamation

pub mod policy;
pub mod state;

#[cfg(kani)]
pub mod proofs;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Public Re-exports
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// Policy exports: Decision functions
pub use policy::{PoolPolicy, StorageStrategy};

// State exports: Metrics and health assessment
pub use state::{HealthStatus, PoolHealthCheck, PoolSnapshot};
