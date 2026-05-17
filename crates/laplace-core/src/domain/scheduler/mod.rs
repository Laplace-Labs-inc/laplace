//! Scheduler Module - Thread-Aware Task Scheduling
//!
//! This module implements deterministic task scheduling for the Laplace platform,
//! providing both production and verification implementations. The scheduler manages
//! thread states and determines execution order for concurrent tasks while integrating
//! with the platform's global time and entropy services.
//!
//! # Architecture
//!
//! The scheduler is organized into distinct responsibilities. The `SchedulerEngine`
//! contains the core scheduling logic and makes decisions about which thread's tasks
//! to execute. The `SchedulerBackend` trait abstracts storage of thread states,
//! allowing different implementations optimized for production and verification
//! scenarios. Time and entropy flow through the global domain layer, eliminating
//! unnecessary complexity and enabling seamless integration with other platform
//! components.
//!
//! # Module Organization
//!
//! The types module defines fundamental types including `ThreadId`, `ThreadState`,
//! and `SchedulingStrategy`. The traits module specifies the `SchedulerBackend`
//! interface that all storage implementations must satisfy. The engine module
//! contains the main scheduling logic, while production and verification modules
//! provide optimized implementations for their respective use cases.
//!
//! # TLA+ Correspondence
//!
//! The scheduler implements the `SchedulerOracle` specification:
//!
//! ```tla
//! VARIABLES virtualTimeNs, threadStates
//!
//! Init ==
//!     /\ virtualTimeNs = 0
//!     /\ threadStates = [t \in Threads |-> "RUNNABLE"]
//!
//! Next ==
//!     \/ ScheduleEvent(...)
//!     \/ ExecuteNext_Production
//!     \/ ExecuteNext_Verification
//! ```
//!
//! # Usage Example
//!
//! ```ignore
//! use laplace_core::domain::scheduler::{ProductionScheduler, ThreadId, SchedulingStrategy};
//!
//! // Create a scheduler with 4 threads
//! let mut scheduler = ProductionScheduler::new(4, SchedulingStrategy::Production);
//!
//! // Schedule a task for a thread
//! let task = scheduler.schedule_task(
//!     ThreadId::new(0),
//!     100_000_000,  // 100ms from now in nanoseconds
//! ).expect("Schedule failed");
//!
//! // In a real system, execute pending events:
//! // while !scheduler.is_idle() {
//! //     scheduler.execute_next().expect("Execution failed");
//! // }
//! ```

pub mod engine;
#[cfg(feature = "verification")]
pub mod production;
pub mod traits;
pub mod types;

#[cfg(any(test, feature = "twin"))]
pub mod verification;

#[cfg(all(kani, feature = "twin"))]
mod proofs;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Public Re-exports
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// Core types
pub use types::{SchedulerError, SchedulingStrategy, TaskId, ThreadId, ThreadState};

// Backend trait
pub use traits::SchedulerBackend;

// Backend implementations
#[cfg(feature = "verification")]
pub use production::ProductionBackend;

#[cfg(any(test, feature = "twin"))]
pub use verification::VerificationBackend;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Type Aliases for Convenience
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Production scheduler with heap-allocated storage and concurrent access
///
/// This type alias provides a convenient way to instantiate a scheduler
/// configured for production deployment. It uses the `ProductionBackend`
/// for efficient concurrent access to thread states with lock-free reads
/// and fine-grained locking for writes.
///
/// # Example
///
/// ```ignore
/// use laplace_core::domain::scheduler::{ProductionScheduler, ThreadId, SchedulingStrategy};
///
/// let mut scheduler = ProductionScheduler::new(8, SchedulingStrategy::Production);
/// let task = scheduler.schedule_task(ThreadId::new(0), 100_000_000)?;
/// ```
#[cfg(feature = "verification")]
pub type ProductionScheduler = engine::SchedulerEngine<ProductionBackend>;

/// Verification scheduler with stack-allocated storage and bounded capacity
///
/// This type alias provides a convenient way to instantiate a scheduler
/// configured for formal verification. It uses the `VerificationBackend`
/// for bounded model checking with fixed-size arrays that remain entirely
/// on the stack, enabling Kani to exhaustively explore the state space.
///
/// # Example
///
/// ```ignore
/// use laplace_core::domain::scheduler::{VerificationScheduler, ThreadId, SchedulingStrategy};
///
/// let mut scheduler = VerificationScheduler::new(4, SchedulingStrategy::Verification);
/// let task = scheduler.schedule_task(ThreadId::new(0), 100_000_000)?;
/// ```
#[cfg(any(test, feature = "twin"))]
pub type VerificationScheduler = engine::SchedulerEngine<VerificationBackend>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_types_are_exported() {
        let _tid = ThreadId::new(0);
        let _task = TaskId::new(0);
        let _state = ThreadState::Runnable;
        let _strategy = SchedulingStrategy::Production;
    }

    #[test]
    fn test_thread_state_display() {
        assert_eq!(format!("{}", ThreadState::Runnable), "RUNNABLE");
        assert_eq!(format!("{}", ThreadState::Blocked), "BLOCKED");
        assert_eq!(format!("{}", ThreadState::Completed), "COMPLETED");
    }

    #[test]
    fn test_strategy_display() {
        assert_eq!(format!("{}", SchedulingStrategy::Production), "PRODUCTION");
        assert_eq!(
            format!("{}", SchedulingStrategy::Verification),
            "VERIFICATION"
        );
    }

    #[cfg(feature = "verification")]
    #[test]
    fn test_production_scheduler_creation() {
        let _scheduler = ProductionScheduler::new(4, SchedulingStrategy::Production);
    }

    #[cfg(any(test, feature = "twin"))]
    #[test]
    fn test_verification_scheduler_creation() {
        let _scheduler = VerificationScheduler::new(4, SchedulingStrategy::Verification);
    }
}
