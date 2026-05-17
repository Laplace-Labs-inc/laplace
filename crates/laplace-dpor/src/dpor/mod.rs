//! Dynamic Partial Order Reduction (DPOR) for Deterministic State Space Exploration
//!
//! This module provides two complementary approaches to concurrent program verification:
//!
//! # Classic DPOR
//!
//! A stack-based depth-first exploration algorithm that:
//! - Uses vector clocks to track causality
//! - Identifies independent operations to prune equivalent executions
//! - Suitable for explicit state verification and finite-trace analysis
//! - Memory efficient with linear space complexity in execution depth
//!
//! # Ki-DPOR (Intelligent DPOR)
//!
//! An A*-based best-first exploration algorithm that:
//! - Uses heuristic guidance to prioritize likely-to-fail executions
//! - Actively searches for starvation and fairness violations
//! - Detects deadlocks and resource contention patterns
//! - Suitable for finding corner cases and liveness bugs
//!
//! # Architectural Principles
//!
//! The DPOR module adheres to three core principles:
//!
//! 1. **Fractal Integrity**: Each component (vector clock, classic scheduler, Ki state,
//!    Ki scheduler) is independently responsible for a single concern and can be evolved
//!    independently while maintaining clean interfaces.
//!
//! 2. **Native-First**: All algorithms are implemented in pure Rust with zero dependencies
//!    on external verification frameworks. The custom TinyBitSet eliminates heap allocation,
//!    and all data structures are designed for minimal memory overhead.
//!
//! 3. **Deterministic Context**: No implicit state propagation. All operations accept
//!    thread identifiers and resource identifiers explicitly, enabling reproducible
//!    verification runs and seamless debugging.
//!
//! # Feature Gating
//!
//! The entire DPOR module is feature-gated behind `feature = "twin"` to ensure:
//! - Zero runtime overhead in production builds
//! - Clear separation between verification and production concerns
//! - Opt-in verification capabilities for enterprise users
//!
//! # TLA+ Correspondence
//!
//! All major algorithms maintain correspondence with TLA+ specifications embedded
//! in module documentation. This ensures mathematical rigor and enables formal
//! verification of the verification infrastructure itself.

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Module Definitions
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub mod classic;
pub mod ki_scheduler;
pub mod ki_state;
pub mod runner;
pub mod schedule;
pub mod vector_clock;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Constants
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Maximum number of threads supported by DPOR algorithms
///
/// This is carefully chosen to:
/// - Fit within TinyBitSet (64 bits maximum)
/// - Balance between flexibility and practical resource verification
/// - Match typical concurrent system thread counts in enterprise settings
pub const MAX_THREADS: usize = 8;

/// Maximum exploration depth for DPOR algorithms
///
/// This controls:
/// - Stack depth for Classic DPOR
/// - Path length for Ki-DPOR
/// - Memory overhead for state exploration
///
/// For typical verification workloads (verifying 3-5 concurrent components),
/// 20 steps is sufficient to expose most concurrency bugs.
pub const MAX_DEPTH: usize = 20;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Public Re-exports: Classic DPOR
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub use classic::{DporScheduler, DporStats, Operation, StepRecord, TinyBitSet};

pub use vector_clock::VectorClock;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Public Re-exports: Ki-DPOR (Intelligent DPOR)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub use ki_state::{KiState, ThreadStatus};

pub use ki_scheduler::{KiDporScheduler, LivenessViolation};

pub use schedule::Schedule;

pub use runner::DporRunner;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Formal Verification Harnesses
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(kani)]
mod classic_proofs;

#[cfg(kani)]
mod ki_proofs;

#[cfg(kani)]
mod vector_clock_proofs;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants_valid() {
        // Verify that constants meet design constraints
        assert!(MAX_THREADS > 0 && MAX_THREADS <= 8);
        assert!(MAX_DEPTH > 0 && MAX_DEPTH <= 100);

        // MAX_THREADS must fit in TinyBitSet (64 bits)
        assert!(MAX_THREADS <= 64);
    }

    #[test]
    fn test_module_structure() {
        // Verify that all expected types are exported
        let _vec_clock = VectorClock::new();
        let _scheduler = DporScheduler::new(2);
        let _ki_scheduler = KiDporScheduler::new(2, 2);
        let _ki_state = KiState::initial(2, 2);
    }
}
