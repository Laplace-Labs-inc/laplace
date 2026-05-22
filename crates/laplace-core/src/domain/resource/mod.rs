// SPDX-License-Identifier: Apache-2.0
//! Resource Tracking and Deadlock Detection Module
//!
//! This module implements the ResourceOracle specification for tracking resource
//! ownership, waiting queues, and detecting deadlocks in concurrent systems.
//!
//! # Architecture
//!
//! The module uses a zero-cost abstraction pattern that provides different
//! implementations based on the build profile:
//!
//! ## Production Builds (`#[cfg(not(feature = "twin"))]`)
//!
//! Uses `NoOpTracker` which is a zero-cost, no-operation implementation.
//! Every method is inlined and compiled away to nothing, providing:
//! - Literally zero runtime overhead
//! - Zero memory footprint
//! - All calls removed by the compiler
//!
//! This is appropriate for production systems because:
//! 1. Deadlock detection is performed during verification with Axiom
//! 2. Production code is guaranteed correct through verification
//! 3. Runtime deadlock detection would add unnecessary overhead
//! 4. Most production systems rarely encounter deadlocks if tested correctly
//!
//! ## Verification Builds (`#[cfg(feature = "twin")]`)
//!
//! Uses `DetailedTracker` which provides comprehensive tracking:
//! - Full resource ownership state
//! - Waiting queue management
//! - Wait-for graph with cycle detection
//! - Heuristic metrics for Ki-DPOR exploration
//!
//! This enables Axiom to:
//! 1. Detect deadlocks during simulation
//! 2. Report deadlock cycles to the user
//! 3. Feed contention metrics to the Ki-DPOR scheduler
//! 4. Ensure verification coverage of all resource scenarios
//!
//! # TLA+ Correspondence
//!
//! Both implementations conform to the ResourceOracle specification:
//!
//! ```tla
//! ResourceOracle == INSTANCE ResourceOracle WITH
//!     RequestResource <- request,
//!     ReleaseResource <- release,
//!     Finish <- on_finish,
//!     HasCycle <- has_deadlock,
//!     DeadlockedThreads <- deadlocked_threads,
//!     ContentionScore <- contention_score
//! ```
//!
//! # Usage Pattern
//!
//! Code using this module should be written generically over `DefaultTracker`:
//!
//! ```rust,ignore
//! use laplace_core::domain::resource::{DefaultTracker, ThreadId, ResourceId};
//!
//! // Works in both production and verification:
//! let mut tracker = DefaultTracker::new(num_threads, num_resources);
//! tracker.request(ThreadId(0), ResourceId(0))?;
//! tracker.release(ThreadId(0), ResourceId(0))?;
//!
//! // Metrics are always available (zero in production, actual in verification)
//! let contention = tracker.contention_score();
//! let interleaving = tracker.interleaving_score();
//! ```

pub mod guard;
pub mod tracker;
pub mod types;

// Production (zero-cost)
pub mod noop;
pub mod pearce_kelly;
pub mod rag;

// Verification (full tracking)
#[cfg(feature = "twin")]
pub mod detailed;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Public Re-exports: Core Types and Traits
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub use types::{
    ResourceCapacity, ResourceError, ResourceId, ResourceType, ThreadId, ThreadStatus,
};

pub use tracker::{RequestResult, ResourceTracker};

pub use guard::{ResourceGuard, ResourceUsage};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Formal Verification Harnesses
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(all(kani, feature = "twin"))]
mod proofs;

#[cfg(all(kani, feature = "twin"))]
mod rag_proofs;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Feature-Gated DefaultTracker Selection
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Default resource tracker implementation
///
/// This type alias selects the appropriate tracker based on the build profile:
///
/// - **Production** (`#[cfg(not(feature = "twin"))]`): `NoOpTracker`
///   - Zero-cost implementation
///   - All calls optimized away
///   - Zero memory overhead
///
/// - **Verification** (`#[cfg(feature = "twin")]`): `DetailedTracker`
///   - Full state tracking
///   - Deadlock detection
///   - Heuristic metrics
///
/// # Code Pattern
///
/// All code using resource tracking should be written against `DefaultTracker`:
///
/// ```rust,ignore
/// let mut tracker = DefaultTracker::new(8, 4);
/// tracker.request(ThreadId(0), ResourceId(0))?;
/// ```
///
/// This ensures the code:
/// 1. Has zero overhead in production
/// 2. Gets full tracking in verification
/// 3. Needs no conditional compilation
#[cfg(not(feature = "twin"))]
pub use noop::NoOpTracker as DefaultTracker;

#[cfg(feature = "twin")]
pub use detailed::DetailedTracker as DefaultTracker;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Public Constants (for feature = "twin" builds)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Maximum threads for verification-mode detailed tracking
///
/// Only exported when `feature = "twin"` is enabled.
#[cfg(feature = "twin")]
pub use detailed::{MAX_RESOURCES, MAX_THREADS};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resource_id_creation() {
        let rid = ResourceId::new(5);
        assert_eq!(rid.as_usize(), 5);
    }

    #[test]
    fn test_thread_id_creation() {
        let tid = ThreadId::new(3);
        assert_eq!(tid.as_usize(), 3);
    }

    #[test]
    fn test_thread_status_enum() {
        let status = ThreadStatus::Running;
        assert_eq!(status, ThreadStatus::Running);

        let status = ThreadStatus::Blocked;
        assert_eq!(status, ThreadStatus::Blocked);

        let status = ThreadStatus::Finished;
        assert_eq!(status, ThreadStatus::Finished);
    }

    #[test]
    fn test_request_result_enum() {
        assert_eq!(RequestResult::Acquired, RequestResult::Acquired);
        assert_eq!(RequestResult::Blocked, RequestResult::Blocked);
        assert_ne!(RequestResult::Acquired, RequestResult::Blocked);
    }

    #[test]
    fn test_default_tracker_creation() {
        let _tracker = DefaultTracker::new(4, 2);
        // If this compiles, the feature-gated selection is working
    }

    #[test]
    fn test_resource_error_display() {
        let err = ResourceError::InvalidThreadId(ThreadId(10));
        let msg = format!("{}", err);
        assert!(msg.contains("Invalid thread ID"));
    }

    #[cfg(feature = "twin")]
    #[test]
    fn test_detailed_tracker_available_in_twin() {
        use super::detailed::DetailedTracker;
        let tracker = DetailedTracker::new(2, 2);
        assert!(!tracker.has_deadlock());
    }

    #[test]
    fn test_zero_cost_in_production() {
        // In production, NoOpTracker is used and has zero size
        #[cfg(not(feature = "twin"))]
        {
            use std::mem::size_of;
            assert_eq!(size_of::<DefaultTracker>(), 0);
        }
    }
}
