// SPDX-License-Identifier: Apache-2.0
//! Deterministic event tracing for Laplace-Axiom simulation engine.
//!
//! This module provides zero-cost observability for the Laplace platform, capturing
//! causal relationships between events using Lamport timestamps. The tracing system
//! enables:
//!
//! - **Causality Analysis**: Reconstruct happens-before relationships for concurrent events.
//! - **Deterministic Replay**: Replay execution with identical event sequencing.
//! - **Formal Verification**: Verify temporal invariants with TLA+ specifications.
//! - **Production Reports**: Generate detailed simulation analytics with minimal overhead.
//!
//! # Architecture
//!
//! The module is organized into three layers:
//!
//! 1. **Types Layer** (`types`): Core data structures (LamportTimestamp, SimulationEvent, etc.)
//! 2. **Trait Layer** (`traits`): Backend abstraction enabling production and verification modes
//! 3. **Implementation Layer**: Concrete backends (ProductionBackend for performance,
//!    VerificationBackend for Kani-based formal verification)
//!
//! # Key Principles
//!
//! **Fractal Integrity**: Each event is atomically complete with all information needed
//! for analysis, requiring no external context lookups.
//!
//! **Native-First**: Pure Rust implementation with zero heap allocation in hot paths
//! (production backend uses pre-allocated storage).
//!
//! **Deterministic Context**: All timestamp generation is explicit and deterministic,
//! with no implicit state propagation. The tracer integrates with `domain::time` to
//! retrieve current timestamps.
//!
//! # Usage Example
//!
//! ```ignore
//! use laplace_core::domain::tracing::{ProductionTracer, SimulationEvent, MemoryOperation};
//! use laplace_core::domain::memory::Address;
//!
//! // Create a production tracer with 100,000 event capacity
//! let mut tracer = ProductionTracer::with_capacity(100_000);
//!
//! // Log a memory read event
//! tracer.log_read(Address(0x1000), 42).expect("log read");
//!
//! // Log a synchronization event
//! tracer.log_mutex_lock(1, ThreadId::new(0)).expect("log lock");
//!
//! // Retrieve events
//! let events = tracer.get_all_events();
//! println!("Recorded {} events", events.len());
//! ```
//!
//! # Feature Flags
//!
//! - `feature = "twin"`: Enables VerificationBackend and formal verification tools.
//!   Without this flag, only ProductionBackend is available.

pub mod causality;
pub mod engine;
pub mod production;
pub mod traits;
pub mod types;

#[cfg(feature = "twin")]
pub mod verification;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Public Re-exports
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Type definitions for the tracing module.
pub use types::{
    EventMetadata,
    FenceType,

    LamportTimestamp as TracingLamportTimestamp,
    MemoryOperation,
    // Core types
    SimulationEvent,
    SyncEvent,
    ThreadId as TracingThreadId,
    // Constants
    MAX_THREADS,
};

/// Trait definitions and error types.
pub use traits::{TracerBackend, TracingError};

/// The main tracer engine (generic over backend).
pub use engine::{TraceEngine, TraceEngineConfig};

pub use causality::{CausalityGraph, HappensBeforeRelation};

/// Production backend optimized for high-performance recording.
pub use production::ProductionBackend;

/// Verification backend with fixed-size array (available with feature = "twin").
#[cfg(feature = "twin")]
pub use verification::VerificationBackend;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Type Aliases for Convenience
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Production tracer with high-capacity backend.
///
/// Optimized for speed and large event volumes. Suitable for real-world
/// simulation runs requiring millions of event captures with minimal overhead.
pub type ProductionTracer = TraceEngine<ProductionBackend>;

/// Verification tracer with fixed-size array backend (feature = "twin").
///
/// Suitable for formal verification with Kani and testing scenarios.
/// Uses a small fixed-size array to keep verification times tractable.
#[cfg(feature = "twin")]
pub type VerificationTracer = TraceEngine<VerificationBackend>;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Formal Verification Harnesses
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(all(kani, feature = "twin"))]
mod proofs;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Version and Constants
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Tracing format version for backward compatibility.
pub const TRACING_FORMAT_VERSION: u32 = 1;

/// Default maximum number of events for production tracer.
pub const DEFAULT_MAX_EVENTS: usize = 100_000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracing_module_exports() {
        // Verify that key types are publicly accessible
        let _ts = TracingLamportTimestamp::ZERO;
        let _tid = TracingThreadId::new(0);
        let _fence = FenceType::SeqCst;

        // Verify constants are accessible
        let max_threads = std::hint::black_box(MAX_THREADS);
        let default_max_events = std::hint::black_box(DEFAULT_MAX_EVENTS);
        assert!(max_threads > 0);
        assert!(default_max_events > 0);
    }
}
