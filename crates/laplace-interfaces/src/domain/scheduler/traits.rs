//! Scheduler backend trait definition.
//!
//! Defines the abstract interface that all thread-state storage backends must implement.
//! The `SchedulerEngine` in `laplace-core` programs to this trait, enabling production
//! and verification backends to be swapped at compile time.

use super::types::{SchedulerError, ThreadId, ThreadState};

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Unique identifier for a scheduled event tracked by the scheduler.
pub type EventId = u64;

/// Abstract interface for scheduler thread-state storage.
///
/// Implement this trait to provide a concrete storage backend for
/// `SchedulerEngine<B: SchedulerBackend>`.
///
/// # TLA+ Correspondence
/// ```tla
/// VARIABLES threadStates
/// threadStates \in [Threads -> ThreadStates]
/// ```
///
/// # Invariants
///
/// Implementations must maintain:
/// 1. `max_threads()` returns a constant value for the lifetime of the backend.
/// 2. All thread IDs in `[0, max_threads)` have a defined state at all times.
/// 3. Threads start in `Runnable` state on construction.
///
/// # Implementing
///
/// `ProductionBackend` (heap-allocated, concurrent) and `VerificationBackend`
/// (stack-allocated, fixed-size) implement this trait in `laplace-core`.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Scheduler",
        link = "LEP-0010-laplace-interfaces-scheduler_contracts"
    )
)]
pub trait SchedulerBackend {
    /// Create a backend initialized with `num_threads` threads, all in `Runnable` state.
    ///
    /// - `num_threads`: Number of threads to track.
    ///
    /// Returns a fully initialized backend.
    fn new(num_threads: usize) -> Self;

    /// Return the maximum number of threads this backend supports.
    ///
    /// This value is constant for the lifetime of the backend.
    fn max_threads(&self) -> usize;

    /// Return the current state of `thread_id`.
    ///
    /// - `thread_id`: Target thread.
    ///
    /// Returns `Ok(state)` or `Err(SchedulerError::InvalidThreadId)` if out of range.
    ///
    /// # TLA+ Correspondence
    /// `threadStates[t]`
    fn get_state(&self, thread_id: ThreadId) -> Result<ThreadState, SchedulerError>;

    /// Atomically set the state of `thread_id` to `new_state`.
    ///
    /// - `thread_id`: Target thread.
    /// - `new_state`: State to transition into.
    ///
    /// Returns `Ok(previous_state)` or `Err(SchedulerError::InvalidThreadId)`.
    ///
    /// # TLA+ Correspondence
    /// `threadStates' = [threadStates EXCEPT ![t] = new_state]`
    fn set_state(
        &self,
        thread_id: ThreadId,
        new_state: ThreadState,
    ) -> Result<ThreadState, SchedulerError>;

    /// Return `true` if `thread_id` is in the `Runnable` state.
    ///
    /// - `thread_id`: Thread to check.
    ///
    /// Defaults to calling `get_state`; implementations may override for performance.
    #[inline(always)]
    fn is_runnable(&self, thread_id: ThreadId) -> bool {
        self.get_state(thread_id)
            .map(|state| state.is_runnable())
            .unwrap_or(false)
    }

    /// Return `(runnable_count, blocked_count, completed_count)` across all threads.
    fn state_counts(&self) -> (usize, usize, usize);

    /// Reset all threads to `Runnable` state.
    fn reset(&self);

    /// Associate `event_id` with `thread_id` so the scheduler knows which thread owns it.
    ///
    /// - `event_id`: Event to register.
    /// - `thread_id`: Owning thread.
    ///
    /// Returns `Ok(())` or an error if `thread_id` is out of range.
    fn register_event(&self, event_id: EventId, thread_id: ThreadId) -> Result<(), SchedulerError>;

    /// Return the thread that owns `event_id`, or `None` if not registered.
    ///
    /// - `event_id`: Event to look up.
    fn get_event_owner(&self, event_id: EventId) -> Option<ThreadId>;

    /// Remove the registration for `event_id` (called after the event executes).
    ///
    /// - `event_id`: Event to deregister.
    fn unregister_event(&self, event_id: EventId);

    /// Clear all event registrations.
    fn clear_events(&self);

    /// Return the number of registered events whose owning thread is `Runnable`.
    fn count_runnable_events(&self) -> usize;
}
