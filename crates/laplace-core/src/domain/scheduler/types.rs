// SPDX-License-Identifier: Apache-2.0
//! Scheduler type definitions — re-exported from `laplace-interfaces`
//!
//! All canonical types live in `laplace_interfaces::domain::scheduler::types`.
//! This file re-exports them so that code within `laplace-core` can continue to use
//! `crate::domain::scheduler::{ThreadId, TaskId, …}`.

pub use laplace_interfaces::domain::scheduler::types::{
    SchedulerError, SchedulingStrategy, TaskId, ThreadState,
};

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "20_Core_Scheduler",
        link = "LEP-0004-laplace-core-scheduler_determinism"
    )
)]
pub use laplace_interfaces::domain::scheduler::types::ThreadId;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thread_id_creation() {
        let tid = ThreadId::new(42);
        assert_eq!(tid.as_usize(), 42);
    }

    #[test]
    fn test_thread_state_predicates() {
        assert!(ThreadState::Runnable.is_runnable());
        assert!(!ThreadState::Blocked.is_runnable());
        assert!(!ThreadState::Completed.is_runnable());
    }

    #[test]
    fn test_scheduling_strategy_predicates() {
        assert!(SchedulingStrategy::Production.is_production());
        assert!(!SchedulingStrategy::Production.is_verification());
    }

    #[test]
    fn test_error_display() {
        let err = SchedulerError::InvalidThreadId {
            thread_id: ThreadId::new(5),
            max_threads: 4,
        };
        let msg = format!("{}", err);
        assert!(msg.contains("Invalid thread ID"));
    }
}
