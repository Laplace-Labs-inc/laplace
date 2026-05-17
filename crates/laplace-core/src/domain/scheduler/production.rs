// SPDX-License-Identifier: Apache-2.0
//! Production Scheduler Backend
//!
//! The production backend uses concurrent data structures to handle thousands
//! of threads safely and efficiently. It employs DashMap for lock-free concurrent
//! access to thread states and maintains event ownership through a hash map
//! protected by a read-write lock.

use super::traits::{EventId, SchedulerBackend};
use super::types::{SchedulerError, ThreadId, ThreadState};
use dashmap::DashMap;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Production backend using DashMap for concurrent access
///
/// This backend is engineered for real-world workloads where multiple threads
/// may access the scheduler concurrently. DashMap provides lock-free reads and
/// fine-grained locking for writes, enabling high scalability without the
/// contention penalty of a single global lock.
///
/// # Thread Safety
///
/// This backend is safe to share across threads via `Arc`. Multiple threads can
/// call `get_state()` and `set_state()` concurrently without external synchronization.
///
/// # Performance Characteristics
///
/// The `get_state()` operation is particularly efficient, leveraging DashMap's
/// lock-free read path in the common case. The `set_state()` operation acquires
/// a write lock on a single shard, allowing other operations on different shards
/// to proceed in parallel.
pub struct ProductionBackend {
    /// Thread states stored in a concurrent hash map
    thread_states: Arc<DashMap<ThreadId, ThreadState>>,

    /// Maximum number of threads supported
    max_threads: usize,

    /// Event-to-thread mapping for ownership tracking
    event_to_thread: RwLock<HashMap<EventId, ThreadId>>,

    /// O(1) counters — updated atomically on every set_state call
    runnable_count: AtomicUsize,
    blocked_count: AtomicUsize,
    completed_count: AtomicUsize,
}

impl ProductionBackend {
    /// Get the number of threads currently tracked
    ///
    /// This may be less than `max_threads()` if some threads have been
    /// removed or never inserted.
    pub fn active_thread_count(&self) -> usize {
        self.thread_states.len()
    }
}

impl SchedulerBackend for ProductionBackend {
    fn new(num_threads: usize) -> Self {
        let map = DashMap::new();
        for tid in 0..num_threads {
            map.insert(ThreadId::new(tid), ThreadState::Runnable);
        }

        Self {
            thread_states: Arc::new(map),
            max_threads: num_threads,
            event_to_thread: RwLock::new(HashMap::new()),
            // All threads start Runnable
            runnable_count: AtomicUsize::new(num_threads),
            blocked_count: AtomicUsize::new(0),
            completed_count: AtomicUsize::new(0),
        }
    }

    fn max_threads(&self) -> usize {
        self.max_threads
    }

    fn get_state(&self, thread_id: ThreadId) -> Result<ThreadState, SchedulerError> {
        if thread_id.as_usize() >= self.max_threads {
            return Err(SchedulerError::InvalidThreadId {
                thread_id,
                max_threads: self.max_threads,
            });
        }

        self.thread_states
            .get(&thread_id)
            .map(|entry| *entry.value())
            .ok_or(SchedulerError::InvalidThreadId {
                thread_id,
                max_threads: self.max_threads,
            })
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
        if thread_id.as_usize() >= self.max_threads {
            return Err(SchedulerError::InvalidThreadId {
                thread_id,
                max_threads: self.max_threads,
            });
        }

        let old_state = self.thread_states.insert(thread_id, new_state).ok_or(
            SchedulerError::InvalidThreadId {
                thread_id,
                max_threads: self.max_threads,
            },
        )?;

        // O(1) counter update — decrement old, increment new
        match old_state {
            ThreadState::Runnable => self.runnable_count.fetch_sub(1, Ordering::Relaxed),
            ThreadState::Blocked => self.blocked_count.fetch_sub(1, Ordering::Relaxed),
            ThreadState::Completed => self.completed_count.fetch_sub(1, Ordering::Relaxed),
        };
        match new_state {
            ThreadState::Runnable => self.runnable_count.fetch_add(1, Ordering::Relaxed),
            ThreadState::Blocked => self.blocked_count.fetch_add(1, Ordering::Relaxed),
            ThreadState::Completed => self.completed_count.fetch_add(1, Ordering::Relaxed),
        };

        Ok(old_state)
    }

    #[inline]
    fn is_runnable(&self, thread_id: ThreadId) -> bool {
        if thread_id.as_usize() >= self.max_threads {
            return false;
        }

        self.thread_states
            .get(&thread_id)
            .map(|entry| entry.value().is_runnable())
            .unwrap_or(false)
    }

    fn state_counts(&self) -> (usize, usize, usize) {
        // O(1): reads from pre-maintained atomic counters
        (
            self.runnable_count.load(Ordering::Relaxed),
            self.blocked_count.load(Ordering::Relaxed),
            self.completed_count.load(Ordering::Relaxed),
        )
    }

    fn reset(&self) {
        self.thread_states.clear();

        for tid in 0..self.max_threads {
            self.thread_states
                .insert(ThreadId::new(tid), ThreadState::Runnable);
        }

        // Restore counters to initial state (all Runnable)
        self.runnable_count
            .store(self.max_threads, Ordering::Relaxed);
        self.blocked_count.store(0, Ordering::Relaxed);
        self.completed_count.store(0, Ordering::Relaxed);
    }

    fn register_event(&self, event_id: EventId, thread_id: ThreadId) -> Result<(), SchedulerError> {
        let mut map = self.event_to_thread.write();
        map.insert(event_id, thread_id);
        Ok(())
    }

    fn get_event_owner(&self, event_id: EventId) -> Option<ThreadId> {
        let map = self.event_to_thread.read();
        map.get(&event_id).copied()
    }

    fn unregister_event(&self, event_id: EventId) {
        let mut map = self.event_to_thread.write();
        map.remove(&event_id);
    }

    fn clear_events(&self) {
        let mut map = self.event_to_thread.write();
        map.clear();
    }

    fn count_runnable_events(&self) -> usize {
        let map = self.event_to_thread.read();
        map.values()
            .filter(|&&thread_id| self.is_runnable(thread_id))
            .count()
    }
}

impl Clone for ProductionBackend {
    fn clone(&self) -> Self {
        Self {
            thread_states: Arc::clone(&self.thread_states),
            max_threads: self.max_threads,
            event_to_thread: RwLock::new(HashMap::new()),
            runnable_count: AtomicUsize::new(self.runnable_count.load(Ordering::Relaxed)),
            blocked_count: AtomicUsize::new(self.blocked_count.load(Ordering::Relaxed)),
            completed_count: AtomicUsize::new(self.completed_count.load(Ordering::Relaxed)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_production_backend_initialization() {
        let backend = ProductionBackend::new(10);

        assert_eq!(backend.max_threads(), 10);

        for i in 0..10 {
            assert_eq!(
                backend.get_state(ThreadId::new(i)).unwrap(),
                ThreadState::Runnable
            );
        }
    }

    #[test]
    fn test_state_transitions() {
        let backend = ProductionBackend::new(10);

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
        let backend = ProductionBackend::new(10);
        assert!(backend.get_state(ThreadId::new(100)).is_err());
        assert!(backend
            .set_state(ThreadId::new(100), ThreadState::Blocked)
            .is_err());
    }

    #[test]
    fn test_state_counts() {
        let backend = ProductionBackend::new(10);

        let (runnable, blocked, completed) = backend.state_counts();
        assert_eq!(runnable, 10);
        assert_eq!(blocked, 0);
        assert_eq!(completed, 0);

        backend
            .set_state(ThreadId::new(0), ThreadState::Blocked)
            .unwrap();
        backend
            .set_state(ThreadId::new(1), ThreadState::Completed)
            .unwrap();

        let (runnable, blocked, completed) = backend.state_counts();
        assert_eq!(runnable, 8);
        assert_eq!(blocked, 1);
        assert_eq!(completed, 1);
    }

    #[test]
    fn test_reset() {
        let backend = ProductionBackend::new(10);

        backend
            .set_state(ThreadId::new(0), ThreadState::Blocked)
            .unwrap();
        backend
            .set_state(ThreadId::new(1), ThreadState::Completed)
            .unwrap();

        backend.reset();

        let (runnable, blocked, completed) = backend.state_counts();
        assert_eq!(runnable, 10);
        assert_eq!(blocked, 0);
        assert_eq!(completed, 0);
    }

    #[test]
    fn test_event_ownership() {
        let backend = ProductionBackend::new(4);

        let event_id: EventId = 100;
        let thread_id = ThreadId::new(0);

        backend.register_event(event_id, thread_id).unwrap();
        assert_eq!(backend.get_event_owner(event_id), Some(thread_id));

        backend.unregister_event(event_id);
        assert_eq!(backend.get_event_owner(event_id), None);
    }

    #[test]
    fn test_count_runnable_events() {
        let backend = ProductionBackend::new(4);

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
