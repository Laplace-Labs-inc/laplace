//! NoOpTracker - Zero-Cost Production Implementation
//!
//! This module provides a tracking implementation optimized for production builds.
//! All methods are empty stubs that the compiler removes completely.

use super::tracker::*;
use super::types::*;

/// Production tracker with zero runtime cost
///
/// # Zero-Cost Guarantee
///
/// Every method is marked with `#[inline(always)]` and has an empty body.
/// This creates the following compilation behavior:
///
/// ```text
/// Source Code:
///     let mut tracker = NoOpTracker::new(8, 4);
///     tracker.request(ThreadId(0), ResourceId(0))?;
///     tracker.release(ThreadId(0), ResourceId(0))?;
///
/// Compiled Code:
///     <nothing>
/// ```
///
/// The compiler inlines all calls and optimizes away the empty bodies,
/// resulting in zero instructions at runtime. This is achieved through:
///
/// 1. **Zero fields** - The struct has no data members
/// 2. **Empty bodies** - Each method contains no logic
/// 3. **Inline attributes** - Forces function inlining at each call site
/// 4. **Copy semantics** - Stack-only, no heap allocation
///
/// # Rationale
///
/// In production systems, deadlock detection and resource tracking are not
/// needed at runtime. Correctness is ensured through verification in the
/// Axiom simulator. Production code can therefore use this no-operation
/// implementation, paying zero cost for the abstraction.
///
/// # Example
///
/// ```rust,ignore
/// // Production build (no feature = "twin")
/// use laplace_core::domain::resource::DefaultTracker;
///
/// let mut tracker = DefaultTracker::new(8, 4);
/// tracker.request(ThreadId(0), ResourceId(0))?;
/// // Compiled result: NOTHING (zero instructions)
/// ```
#[derive(Debug, Clone, Copy)]
pub struct NoOpTracker;

impl ResourceTracker for NoOpTracker {
    /// Create a new no-op tracker
    ///
    /// Accepts any thread and resource counts but does nothing with them.
    /// This allows NoOpTracker to be a drop-in replacement for DetailedTracker.
    #[inline(always)]
    fn new(_num_threads: usize, _num_resources: usize) -> Self {
        Self
    }

    /// Request a resource (always succeeds immediately)
    ///
    /// Returns Ok(RequestResult::Acquired) unconditionally.
    /// No state changes occur.
    #[inline(always)]
    fn request(
        &mut self,
        _thread: ThreadId,
        _resource: ResourceId,
    ) -> Result<RequestResult, ResourceError> {
        Ok(RequestResult::Acquired)
    }

    /// Release a resource (always succeeds)
    ///
    /// Returns Ok(()) unconditionally.
    /// No state changes occur.
    #[inline(always)]
    fn release(&mut self, _thread: ThreadId, _resource: ResourceId) -> Result<(), ResourceError> {
        Ok(())
    }

    /// Mark thread as finished (always succeeds)
    ///
    /// Returns Ok(()) unconditionally.
    /// No state changes occur.
    #[inline(always)]
    fn on_finish(&mut self, _thread: ThreadId) -> Result<(), ResourceError> {
        Ok(())
    }

    /// Check for deadlock (always returns false)
    ///
    /// Production code assumes no deadlocks occur.
    /// Deadlock detection is only available in verification builds.
    #[inline(always)]
    fn has_deadlock(&self) -> bool {
        false
    }

    /// Get deadlocked threads (always returns empty)
    ///
    /// Production code has no deadlocks to report.
    #[inline(always)]
    fn deadlocked_threads(&self) -> Vec<ThreadId> {
        Vec::new()
    }

    /// Get contention score (always returns zero)
    ///
    /// Production code does not track contention metrics.
    /// Heuristics are only computed during verification.
    #[inline(always)]
    fn contention_score(&self) -> u32 {
        0
    }

    /// Get interleaving score (always returns zero)
    ///
    /// Production code does not track context switches.
    /// Interleaving metrics are only computed during verification.
    #[inline(always)]
    fn interleaving_score(&self) -> u32 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_noop_compiles_and_runs() {
        let mut tracker = NoOpTracker::new(8, 4);

        // All operations should succeed with no side effects
        assert_eq!(
            tracker.request(ThreadId(0), ResourceId(0)).unwrap(),
            RequestResult::Acquired
        );

        assert_eq!(
            tracker.request(ThreadId(1), ResourceId(1)).unwrap(),
            RequestResult::Acquired
        );

        tracker.release(ThreadId(0), ResourceId(0)).unwrap();
        tracker.release(ThreadId(1), ResourceId(1)).unwrap();

        tracker.on_finish(ThreadId(0)).unwrap();
        tracker.on_finish(ThreadId(1)).unwrap();

        // No deadlock can be detected
        assert!(!tracker.has_deadlock());
        assert_eq!(tracker.deadlocked_threads().len(), 0);

        // No metrics are computed
        assert_eq!(tracker.contention_score(), 0);
        assert_eq!(tracker.interleaving_score(), 0);
    }

    #[test]
    fn test_noop_size() {
        // Verify that NoOpTracker has zero size
        use std::mem::size_of;
        assert_eq!(size_of::<NoOpTracker>(), 0);
    }

    #[test]
    fn test_noop_drop_is_free() {
        // Verify that dropping is free
        let tracker = NoOpTracker::new(100, 100);
        let _ = tracker;
        // No cleanup needed
    }
}
