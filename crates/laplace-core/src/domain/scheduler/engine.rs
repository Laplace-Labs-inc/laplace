//! SchedulerEngine - Pure Scheduling Logic
//!
//! The engine implements the scheduling algorithm from the TLA+ specification,
//! delegating state management to a pluggable backend and time/randomness to
//! the platform's global services.
//!
//! # Architecture
//!
//! ```text
//! SchedulerEngine<B>
//!   ├─ backend: B              (thread states - pluggable)
//!   ├─ domain::now_ns()        (time - global)
//!   └─ domain::next_random*()  (entropy - global)
//! ```
//!
//! # TLA+ Correspondence
//!
//! ```tla
//! VARIABLES virtualTimeNs, threadStates
//!
//! ScheduleEvent(thread, delay) ==
//!     /\ threadStates[thread] = "RUNNABLE"
//!     /\ thread \in Threads
//!     /\ RegisterEvent(thread, delay)
//!
//! ExecuteNext ==
//!     \/ ExecuteNext_Production
//!     \/ ExecuteNext_Verification
//! ```

use super::traits::{EventId, SchedulerBackend};
use super::types::{SchedulerError, SchedulingStrategy, TaskId, ThreadId, ThreadState};

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// SchedulerEngine - Thread-aware scheduling with global time and entropy
///
/// The engine is generic only over the backend, keeping it simple and focused.
/// Time and entropy flow through the global domain layer, eliminating the need
/// for clock and randomness generics.
///
/// # Example
///
/// ```ignore
/// use laplace_core::domain::scheduler::{
///     engine::SchedulerEngine,
///     production::ProductionBackend,
///     SchedulingStrategy,
/// };
///
/// let backend = ProductionBackend::new(4);
/// let mut scheduler = SchedulerEngine::new(backend, 4, SchedulingStrategy::Production);
///
/// // Schedule a task
/// let task = scheduler.schedule_task(
///     ThreadId::new(0),
///     100_000_000,  // 100ms in nanoseconds
/// ).unwrap();
/// ```
pub struct SchedulerEngine<B: SchedulerBackend> {
    /// Backend for thread state storage
    backend: B,

    /// Scheduling strategy (deterministic vs. non-deterministic)
    /// Kept for interface stability; strategy configuration may be used in future implementations.
    #[allow(dead_code)]
    strategy: SchedulingStrategy,

    /// Next task ID allocator
    next_task_id: TaskId,
}

impl<B: SchedulerBackend> SchedulerEngine<B> {
    /// Create a new scheduler engine
    ///
    /// All threads are initialized to RUNNABLE state by the backend.
    ///
    /// # Arguments
    ///
    /// - `num_threads`: Number of threads to support
    /// - `strategy`: Scheduling strategy (PRODUCTION or VERIFICATION)
    pub fn new(num_threads: usize, strategy: SchedulingStrategy) -> Self {
        let backend = B::new(num_threads);

        Self {
            backend,
            strategy,
            next_task_id: TaskId::new(0),
        }
    }

    /// Get reference to the backend
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Get current virtual time in nanoseconds
    ///
    /// Uses the global domain time service.
    #[inline(always)]
    pub fn now_ns(&self) -> u64 {
        crate::domain::now_ns() as u64
    }

    /// Get number of threads
    pub fn num_threads(&self) -> usize {
        self.backend.max_threads()
    }

    /// Get thread state
    pub fn get_thread_state(&self, thread_id: ThreadId) -> Result<ThreadState, SchedulerError> {
        self.backend.get_state(thread_id)
    }

    /// Set thread state
    ///
    /// # State Transitions
    ///
    /// Valid transitions include RUNNABLE → BLOCKED, BLOCKED → RUNNABLE,
    /// and RUNNABLE → COMPLETED. The backend does not validate these transitions;
    /// that responsibility belongs to higher-level logic.
    pub fn set_thread_state(
        &mut self,
        thread_id: ThreadId,
        new_state: ThreadState,
    ) -> Result<ThreadState, SchedulerError> {
        self.backend.set_state(thread_id, new_state)
    }

    /// Schedule a task for a thread
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// ScheduleEvent(thread, delay) ==
    ///     /\ virtualTimeNs + delay <= MaxTimeNs
    ///     /\ thread \in Threads
    ///     /\ threadStates[thread] = "RUNNABLE"
    ///     /\ RegisterEvent(thread, delay)
    /// ```
    ///
    /// # Arguments
    ///
    /// - `thread_id`: Which thread this task belongs to
    /// - `delay_ns`: When to execute (nanoseconds from now)
    ///
    /// # Returns
    ///
    /// A `TaskId` for tracking this scheduled task, or an error if preconditions fail.
    ///
    /// # Errors
    ///
    /// Returns `InvalidThreadId` if the thread does not exist, `InvalidThreadState`
    /// if the thread is not RUNNABLE, or `TimeOverflow` if the delay would exceed
    /// time limits.
    pub fn schedule_task(
        &mut self,
        thread_id: ThreadId,
        delay_ns: u64,
    ) -> Result<TaskId, SchedulerError> {
        // Verify thread exists and is RUNNABLE
        let thread_state = self.backend.get_state(thread_id)?;

        if !thread_state.is_runnable() {
            return Err(SchedulerError::InvalidThreadState {
                thread_id,
                current_state: thread_state,
                expected_state: ThreadState::Runnable,
            });
        }

        // Calculate scheduled time
        let current_time = self.now_ns();
        let scheduled_time =
            current_time
                .checked_add(delay_ns)
                .ok_or(SchedulerError::TimeOverflow {
                    current_time_ns: current_time,
                    delay_ns,
                    max_time_ns: u64::MAX,
                })?;

        // Create event ID from current time and randomness
        let event_id = self.generate_event_id(scheduled_time);

        // Register event ownership
        self.backend.register_event(event_id, thread_id)?;

        // Allocate and return task ID
        let task_id = self.next_task_id;
        self.next_task_id = TaskId::new(self.next_task_id.as_usize() + 1);

        Ok(task_id)
    }

    /// Execute the next runnable event
    ///
    /// This method checks the clock for pending events and executes the next one
    /// whose owning thread is in RUNNABLE state. If the next event belongs to a
    /// blocked thread, it is skipped and the next event is checked.
    ///
    /// # Returns
    ///
    /// `Some(event_id)` if an event was executed, `None` if no runnable events
    /// remain (either queue is empty or all events belong to blocked threads).
    pub fn execute_next(&mut self) -> Result<Option<EventId>, SchedulerError> {
        // In this simplified version, we just acknowledge that an event would be
        // executed. In a full implementation, we would:
        // 1. Get next event from global clock queue
        // 2. Look up owning thread
        // 3. Check if thread is RUNNABLE
        // 4. Execute or skip accordingly

        // For now, return None to indicate no events pending
        Ok(None)
    }

    /// Get thread state statistics
    pub fn thread_state_counts(&self) -> (usize, usize, usize) {
        self.backend.state_counts()
    }

    /// Check if scheduler is idle
    ///
    /// Returns true if there are no runnable events (either no events in queue
    /// or all events belong to blocked threads).
    pub fn is_idle(&self) -> bool {
        self.backend.count_runnable_events() == 0
    }

    /// Reset scheduler to initial state
    pub fn reset(&mut self) {
        self.backend.reset();
        self.backend.clear_events();
        self.next_task_id = TaskId::new(0);
    }

    /// Generate a unique event ID from time and entropy
    ///
    /// Uses the current scheduled time combined with random data to create
    /// an event ID that is both timestamped and unique.
    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(
            layer = "20_Core_Scheduler",
            link = "LEP-0004-laplace-core-scheduler_determinism"
        )
    )]
    fn generate_event_id(&self, scheduled_time: u64) -> EventId {
        // Combine time with random data for uniqueness
        let random_component = crate::domain::next_random_u64();
        scheduled_time ^ random_component
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(not(kani), feature = "verification"))]
    mod production_tests {
        use super::*;
        use crate::domain::scheduler::production::ProductionBackend;

        type TestEngine = SchedulerEngine<ProductionBackend>;

        #[test]
        fn test_engine_creation() {
            let engine = TestEngine::new(4, SchedulingStrategy::Production);

            assert_eq!(engine.num_threads(), 4);

            let (runnable, blocked, completed) = engine.thread_state_counts();
            assert_eq!(runnable, 4);
            assert_eq!(blocked, 0);
            assert_eq!(completed, 0);
        }

        #[test]
        fn test_schedule_task() {
            let mut engine = TestEngine::new(4, SchedulingStrategy::Production);

            let task = engine
                .schedule_task(ThreadId::new(0), 100_000_000)
                .expect("Schedule failed");

            assert_eq!(task.as_usize(), 0);
        }

        #[test]
        fn test_schedule_invalid_thread() {
            let mut engine = TestEngine::new(4, SchedulingStrategy::Production);

            let result = engine.schedule_task(ThreadId::new(10), 100_000_000);
            assert!(result.is_err());
        }

        #[test]
        fn test_schedule_blocked_thread() {
            let mut engine = TestEngine::new(4, SchedulingStrategy::Production);

            engine
                .set_thread_state(ThreadId::new(0), ThreadState::Blocked)
                .expect("Set state failed");

            let result = engine.schedule_task(ThreadId::new(0), 100_000_000);
            assert!(result.is_err());
        }

        #[test]
        fn test_thread_state_transitions() {
            let mut engine = TestEngine::new(4, SchedulingStrategy::Production);

            // RUNNABLE -> BLOCKED
            let prev = engine
                .set_thread_state(ThreadId::new(0), ThreadState::Blocked)
                .expect("Set state failed");
            assert_eq!(prev, ThreadState::Runnable);

            let (runnable, blocked, completed) = engine.thread_state_counts();
            assert_eq!(runnable, 3);
            assert_eq!(blocked, 1);
            assert_eq!(completed, 0);

            // BLOCKED -> COMPLETED
            let prev = engine
                .set_thread_state(ThreadId::new(0), ThreadState::Completed)
                .expect("Set state failed");
            assert_eq!(prev, ThreadState::Blocked);

            let (runnable, blocked, completed) = engine.thread_state_counts();
            assert_eq!(runnable, 3);
            assert_eq!(blocked, 0);
            assert_eq!(completed, 1);
        }

        #[test]
        fn test_reset() {
            let mut engine = TestEngine::new(4, SchedulingStrategy::Production);

            engine
                .set_thread_state(ThreadId::new(0), ThreadState::Blocked)
                .expect("Set state failed");

            engine.reset();

            let (runnable, blocked, completed) = engine.thread_state_counts();
            assert_eq!(runnable, 4);
            assert_eq!(blocked, 0);
            assert_eq!(completed, 0);
        }

        #[test]
        fn test_is_idle() {
            let engine = TestEngine::new(4, SchedulingStrategy::Production);
            assert!(engine.is_idle());
        }
    }

    #[cfg(any(test, feature = "twin"))]
    mod verification_tests {
        use super::*;
        use crate::domain::scheduler::verification::VerificationBackend;

        type TestEngine = SchedulerEngine<VerificationBackend>;

        #[test]
        fn test_verification_engine_creation() {
            let engine = TestEngine::new(4, SchedulingStrategy::Verification);

            assert_eq!(engine.num_threads(), 4);

            let (runnable, blocked, completed) = engine.thread_state_counts();
            assert_eq!(runnable, 4);
            assert_eq!(blocked, 0);
            assert_eq!(completed, 0);
        }

        #[test]
        fn test_verification_schedule_task() {
            let mut engine = TestEngine::new(4, SchedulingStrategy::Verification);

            let task = engine
                .schedule_task(ThreadId::new(0), 100_000_000)
                .expect("Schedule failed");

            assert_eq!(task.as_usize(), 0);
        }
    }
}
