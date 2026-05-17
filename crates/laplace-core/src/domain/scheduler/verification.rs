//! Verification Scheduler Backend
//!
//! The verification backend is optimized for formal verification with bounded
//! model checking. It uses fixed-size arrays stored on the stack to maintain
//! a tractable state space for tools like Kani, eliminating the complexity
//! of concurrent data structures and heap allocation.

use super::traits::{EventId, SchedulerBackend};
use super::types::{SchedulerError, ThreadId, ThreadState};
use std::cell::{Cell, RefCell};

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Maximum number of threads in verification mode
///
/// This bound is intentionally conservative to keep the state space manageable
/// for exhaustive verification. With 4 threads and 3 possible states per thread,
/// we have 81 possible thread state combinations, which is well within Kani's
/// capabilities.
pub const MAX_THREADS: usize = 4;

/// Maximum number of concurrent events
pub const MAX_EVENTS: usize = 8;

/// Verification backend using fixed arrays
///
/// This backend provides a stack-allocated implementation suitable for formal
/// verification. All memory is allocated at construction time on the stack,
/// allowing Kani to fully explore the bounded state space without dealing with
/// the complexity of heap allocation and dynamic growth.
///
/// The interior mutability pattern via `RefCell` allows the backend to satisfy
/// the `SchedulerBackend` trait methods which take `&self` while still enabling
/// state modification. This is safe in the single-threaded verification context.
pub struct VerificationBackend {
    /// Thread states stored in a fixed array
    thread_states: RefCell<[ThreadState; MAX_THREADS]>,

    /// Number of threads actually being used
    num_threads: usize,

    /// Event-to-thread mapping using a fixed-size array
    event_to_thread: RefCell<[(EventId, ThreadId); MAX_EVENTS]>,

    /// Count of registered events
    event_count: RefCell<usize>,

    /// O(1) counters — updated on every set_state call via Cell<usize>
    runnable_count: Cell<usize>,
    blocked_count: Cell<usize>,
    completed_count: Cell<usize>,
}

impl VerificationBackend {
    /// Get a reference to the thread states array
    ///
    /// This is primarily useful for testing and inspection. Most code should
    /// use the `SchedulerBackend` trait methods instead.
    pub fn states(&self) -> std::cell::Ref<'_, [ThreadState; MAX_THREADS]> {
        self.thread_states.borrow()
    }

    /// Get the number of threads being used
    ///
    /// This may be less than `MAX_THREADS` if the backend was created with
    /// a smaller thread count.
    pub fn num_threads(&self) -> usize {
        self.num_threads
    }
}

impl SchedulerBackend for VerificationBackend {
    fn new(num_threads: usize) -> Self {
        assert!(
            num_threads <= MAX_THREADS,
            "VerificationBackend supports at most {} threads, requested {}",
            MAX_THREADS,
            num_threads
        );

        Self {
            thread_states: RefCell::new([ThreadState::Runnable; MAX_THREADS]),
            num_threads,
            event_to_thread: RefCell::new([(0, ThreadId::new(0)); MAX_EVENTS]),
            event_count: RefCell::new(0),
            // All threads start Runnable
            runnable_count: Cell::new(num_threads),
            blocked_count: Cell::new(0),
            completed_count: Cell::new(0),
        }
    }

    fn max_threads(&self) -> usize {
        self.num_threads
    }

    fn get_state(&self, thread_id: ThreadId) -> Result<ThreadState, SchedulerError> {
        if thread_id.as_usize() >= self.num_threads {
            return Err(SchedulerError::InvalidThreadId {
                thread_id,
                max_threads: self.num_threads,
            });
        }

        let states = self.thread_states.borrow();
        Ok(states[thread_id.as_usize()])
    }

    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(
            layer = "20_Core_Scheduler",
            link = "LEP-0004-laplace-core-scheduler_determinism"
        )
    )]
    fn set_state(
        &self,
        thread_id: ThreadId,
        new_state: ThreadState,
    ) -> Result<ThreadState, SchedulerError> {
        if thread_id.as_usize() >= self.num_threads {
            return Err(SchedulerError::InvalidThreadId {
                thread_id,
                max_threads: self.num_threads,
            });
        }

        let mut states = self.thread_states.borrow_mut();
        let old_state = states[thread_id.as_usize()];
        states[thread_id.as_usize()] = new_state;

        // O(1) counter update
        match old_state {
            ThreadState::Runnable => self.runnable_count.set(self.runnable_count.get() - 1),
            ThreadState::Blocked => self.blocked_count.set(self.blocked_count.get() - 1),
            ThreadState::Completed => self.completed_count.set(self.completed_count.get() - 1),
        }
        match new_state {
            ThreadState::Runnable => self.runnable_count.set(self.runnable_count.get() + 1),
            ThreadState::Blocked => self.blocked_count.set(self.blocked_count.get() + 1),
            ThreadState::Completed => self.completed_count.set(self.completed_count.get() + 1),
        }

        Ok(old_state)
    }

    #[inline]
    fn is_runnable(&self, thread_id: ThreadId) -> bool {
        if thread_id.as_usize() >= self.num_threads {
            return false;
        }

        let states = self.thread_states.borrow();
        states[thread_id.as_usize()].is_runnable()
    }

    fn state_counts(&self) -> (usize, usize, usize) {
        // O(1): reads from pre-maintained Cell counters
        (
            self.runnable_count.get(),
            self.blocked_count.get(),
            self.completed_count.get(),
        )
    }

    fn reset(&self) {
        let mut states = self.thread_states.borrow_mut();

        for state in states.iter_mut() {
            *state = ThreadState::Runnable;
        }

        // Restore counters to initial state (all Runnable)
        self.runnable_count.set(self.num_threads);
        self.blocked_count.set(0);
        self.completed_count.set(0);
    }

    fn register_event(&self, event_id: EventId, thread_id: ThreadId) -> Result<(), SchedulerError> {
        let mut count = self.event_count.borrow_mut();

        if *count >= MAX_EVENTS {
            return Err(SchedulerError::QueueFull {
                max_events: MAX_EVENTS,
                attempted: *count + 1,
            });
        }

        let mut events = self.event_to_thread.borrow_mut();
        events[*count] = (event_id, thread_id);
        *count += 1;

        Ok(())
    }

    fn get_event_owner(&self, event_id: EventId) -> Option<ThreadId> {
        let count = *self.event_count.borrow();
        let events = self.event_to_thread.borrow();

        for i in 0..count {
            if events[i].0 == event_id {
                return Some(events[i].1);
            }
        }

        None
    }

    fn unregister_event(&self, event_id: EventId) {
        let mut count = self.event_count.borrow_mut();
        let mut events = self.event_to_thread.borrow_mut();

        for i in 0..*count {
            if events[i].0 == event_id {
                *count -= 1;
                events[i] = events[*count];
                return;
            }
        }
    }

    fn clear_events(&self) {
        let mut count = self.event_count.borrow_mut();
        *count = 0;
    }

    fn count_runnable_events(&self) -> usize {
        let count = *self.event_count.borrow();
        let events = self.event_to_thread.borrow();

        let mut runnable_count = 0;
        for i in 0..count {
            let thread_id = events[i].1;
            if self.is_runnable(thread_id) {
                runnable_count += 1;
            }
        }
        runnable_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verification_initialization() {
        let backend = VerificationBackend::new(4);

        assert_eq!(backend.max_threads(), 4);

        for i in 0..4 {
            assert_eq!(
                backend.get_state(ThreadId::new(i)).unwrap(),
                ThreadState::Runnable
            );
        }
    }

    #[test]
    fn test_state_transitions() {
        let backend = VerificationBackend::new(4);

        let prev = backend
            .set_state(ThreadId::new(0), ThreadState::Blocked)
            .unwrap();
        assert_eq!(prev, ThreadState::Runnable);
        assert_eq!(
            backend.get_state(ThreadId::new(0)).unwrap(),
            ThreadState::Blocked
        );
    }

    #[test]
    fn test_out_of_bounds() {
        let backend = VerificationBackend::new(4);

        assert!(backend.get_state(ThreadId::new(10)).is_err());
        assert!(backend
            .set_state(ThreadId::new(10), ThreadState::Runnable)
            .is_err());
    }

    #[test]
    #[should_panic(expected = "supports at most")]
    fn test_max_threads_exceeded() {
        VerificationBackend::new(MAX_THREADS + 1);
    }

    #[test]
    fn test_state_counts() {
        let backend = VerificationBackend::new(4);

        let (runnable, blocked, completed) = backend.state_counts();
        assert_eq!(runnable, 4);
        assert_eq!(blocked, 0);
        assert_eq!(completed, 0);

        backend
            .set_state(ThreadId::new(0), ThreadState::Blocked)
            .unwrap();
        backend
            .set_state(ThreadId::new(1), ThreadState::Completed)
            .unwrap();

        let (runnable, blocked, completed) = backend.state_counts();
        assert_eq!(runnable, 2);
        assert_eq!(blocked, 1);
        assert_eq!(completed, 1);
    }

    #[test]
    fn test_reset() {
        let backend = VerificationBackend::new(4);

        backend
            .set_state(ThreadId::new(0), ThreadState::Blocked)
            .unwrap();
        backend
            .set_state(ThreadId::new(1), ThreadState::Completed)
            .unwrap();

        backend.reset();

        let (runnable, blocked, completed) = backend.state_counts();
        assert_eq!(runnable, 4);
        assert_eq!(blocked, 0);
        assert_eq!(completed, 0);
    }

    #[test]
    fn test_event_ownership() {
        let backend = VerificationBackend::new(4);

        let event_id: EventId = 100;
        let thread_id = ThreadId::new(0);

        backend.register_event(event_id, thread_id).unwrap();
        assert_eq!(backend.get_event_owner(event_id), Some(thread_id));

        backend.unregister_event(event_id);
        assert_eq!(backend.get_event_owner(event_id), None);
    }

    #[test]
    fn test_event_queue_full() {
        let backend = VerificationBackend::new(4);

        for i in 0..MAX_EVENTS {
            backend
                .register_event(i as EventId, ThreadId::new(0))
                .expect("Register should succeed");
        }

        let result = backend.register_event(MAX_EVENTS as EventId, ThreadId::new(0));
        assert!(result.is_err());
    }

    #[test]
    fn test_count_runnable_events() {
        let backend = VerificationBackend::new(4);

        backend.register_event(100, ThreadId::new(0)).unwrap();
        backend.register_event(101, ThreadId::new(1)).unwrap();
        backend.register_event(102, ThreadId::new(2)).unwrap();

        assert_eq!(backend.count_runnable_events(), 3);

        backend
            .set_state(ThreadId::new(0), ThreadState::Blocked)
            .unwrap();
        assert_eq!(backend.count_runnable_events(), 2);

        backend
            .set_state(ThreadId::new(1), ThreadState::Completed)
            .unwrap();
        assert_eq!(backend.count_runnable_events(), 1);
    }
}
