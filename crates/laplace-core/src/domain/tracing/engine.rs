//! Core trace engine for recording simulation events with Lamport timestamps.
//!
//! This module provides the `TraceEngine`, the primary API for event recording.
//! It maintains per-thread Lamport clock state and provides convenience methods
//! for common event types (memory operations, synchronization, thread management).
//!
//! **Principle**: Deterministic Context — all timestamp generation is explicit
//! and deterministic, with no implicit state propagation.

use super::traits::{TracerBackend, TracingError};
use super::types::{
    EventMetadata, FenceType, LamportTimestamp, MemoryOperation, SimulationEvent, SyncEvent,
    ThreadId, MAX_THREADS,
};
use crate::domain::memory::Address;
use std::fmt;

/// Per-thread state maintained by the TraceEngine.
///
/// This structure tracks the Lamport clock and sequence number for a single thread,
/// enabling the engine to assign monotonically increasing timestamps to that thread's events.
#[derive(Debug, Clone, Copy)]
struct ThreadState {
    /// The last Lamport timestamp issued to this thread.
    last_timestamp: LamportTimestamp,
    /// Sequence number for events within this thread (for total ordering).
    seq_num: u64,
}

impl ThreadState {
    /// Create a new thread state with initial clock value of zero.
    #[inline(always)]
    const fn new() -> Self {
        Self {
            last_timestamp: LamportTimestamp::ZERO,
            seq_num: 0,
        }
    }

    /// Generate the next timestamp for this thread and increment the clock.
    ///
    /// This increments the local clock, increments the sequence number, and
    /// returns the new timestamp.
    #[inline]
    fn next_timestamp(&mut self) -> LamportTimestamp {
        self.last_timestamp.increment();
        self.seq_num = self.seq_num.wrapping_add(1);
        self.last_timestamp
    }

    /// Synchronize this thread's clock with a received timestamp.
    ///
    /// Implements the Lamport clock rule: `new_ts = max(local, received) + 1`.
    /// This ensures that the thread's clock advances when receiving messages
    /// from other threads.
    #[inline]
    fn sync_with(&mut self, received_timestamp: LamportTimestamp) {
        self.last_timestamp.sync(received_timestamp);
        self.seq_num = self.seq_num.wrapping_add(1);
    }
}

/// Configuration for the TraceEngine.
///
/// This structure controls behavior of the engine during event recording,
/// such as whether to validate causality invariants on append.
#[derive(Debug, Clone, Copy)]
pub struct TraceEngineConfig {
    /// Whether to validate causality on every append.
    ///
    /// When enabled, the engine checks that timestamps are monotonically
    /// increasing per thread. This has a small performance cost but catches
    /// bugs early. Recommended for development/testing.
    pub validate_causality: bool,
}

impl Default for TraceEngineConfig {
    fn default() -> Self {
        Self {
            validate_causality: cfg!(debug_assertions),
        }
    }
}

/// The main event trace engine.
///
/// The TraceEngine is the primary interface for recording simulation events.
/// It maintains per-thread Lamport clock state and delegates storage to a
/// backend implementation (production or verification).
///
/// # Type Parameters
///
/// * `B` - The storage backend (ProductionBackend or VerificationBackend).
///
/// # Memory Layout
///
/// - thread_states: 16 * 16 = 256 bytes (16 threads, each ThreadState is 16 bytes)
/// - backend: B-specific (typically Vec or array)
/// - config: 1 byte
/// - Total: ~300+ bytes for ProductionBackend, ~4 KB for VerificationBackend
///
/// # Thread Safety
///
/// The TraceEngine itself is not thread-safe. Concurrent access should be
/// synchronized by the caller using `Mutex<TraceEngine>` or similar.
/// This design keeps the hot path lock-free for single-threaded usage.
///
/// # Example
///
/// ```ignore
/// use laplace_core::domain::tracing::{ProductionBackend, TraceEngine, TraceEngineConfig};
/// use laplace_core::domain::memory::Address;
///
/// let backend = ProductionBackend::with_capacity(100_000);
/// let config = TraceEngineConfig::default();
/// let mut tracer = TraceEngine::new(backend, config);
///
/// let tid = ThreadId::new(0);
///
/// // Log a memory read
/// tracer.log_read(tid, Address(0x1000), 42)?;
///
/// // Log a synchronization event
/// tracer.log_mutex_lock(tid, 1)?;
///
/// // Retrieve events
/// let events = tracer.get_all_events();
/// println!("Recorded {} events", events.len());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct TraceEngine<B: TracerBackend> {
    /// Per-thread Lamport clock state.
    thread_states: [ThreadState; MAX_THREADS],
    /// Storage backend (Vec-based for production, array-based for verification).
    backend: B,
    /// Configuration options.
    config: TraceEngineConfig,
}

impl<B: TracerBackend> TraceEngine<B> {
    /// Create a new trace engine with the given backend and configuration.
    ///
    /// # Arguments
    ///
    /// * `backend` - The storage backend to use for recording events.
    /// * `config` - Configuration options (e.g., causality validation).
    ///
    /// # Example
    ///
    /// ```ignore
    /// let backend = ProductionBackend::with_capacity(100_000);
    /// let tracer = TraceEngine::new(backend, Default::default());
    /// ```
    pub fn new(backend: B, config: TraceEngineConfig) -> Self {
        Self {
            thread_states: [ThreadState::new(); MAX_THREADS],
            backend,
            config,
        }
    }

    /// Generate metadata for a new event from the given thread.
    ///
    /// This increments the thread's Lamport clock, updates the backend's
    /// global timestamp, and returns the new metadata.
    ///
    /// # Errors
    ///
    /// Returns `TracingError::InvalidThreadId` if the thread ID is out of bounds.
    #[inline]
    fn next_meta(&mut self, thread_id: ThreadId) -> Result<EventMetadata, TracingError> {
        let idx = thread_id.as_index();
        if idx >= MAX_THREADS {
            return Err(TracingError::InvalidThreadId(thread_id.0));
        }

        let state = &mut self.thread_states[idx];
        let ts = state.next_timestamp();
        let seq = state.seq_num;

        self.backend.update_global_timestamp(ts);

        Ok(EventMetadata::new(ts, thread_id, seq))
    }

    /// Append an event to the trace after validating causality (if enabled).
    ///
    /// # Errors
    ///
    /// Returns `TracingError::BufferFull` if the backend buffer is full.
    /// Returns `TracingError::CausalityViolation` if validation is enabled
    /// and the event violates the per-thread timestamp monotonicity invariant.
    #[inline]
    fn append_event(&mut self, event: SimulationEvent) -> Result<(), TracingError> {
        if self.config.validate_causality {
            let meta = event.metadata();
            let idx = meta.thread_id.as_index();
            if idx < MAX_THREADS {
                let last_ts = self.thread_states[idx].last_timestamp;
                // The event you create must match last_ts.
                if meta.timestamp < last_ts && last_ts.0 != 0 {
                    return Err(TracingError::CausalityViolation {
                        expected_min: last_ts,
                        received: meta.timestamp,
                    });
                }
            }
        }

        self.backend.append_event(event)
    }

    /// Log a memory read operation.
    ///
    /// Records a read from the given address with the observed value.
    ///
    /// # Arguments
    ///
    /// * `thread_id` - The thread performing the read.
    /// * `addr` - The memory address being read.
    /// * `value` - The value observed by the read.
    ///
    /// # Errors
    ///
    /// Returns `TracingError::BufferFull` if the backend is at capacity.
    /// Returns `TracingError::InvalidThreadId` if thread_id is out of bounds.
    pub fn log_read(
        &mut self,
        thread_id: ThreadId,
        addr: Address,
        value: u64,
    ) -> Result<(), TracingError> {
        let meta = self.next_meta(thread_id)?;
        let event = SimulationEvent::Memory {
            meta,
            operation: MemoryOperation::Read {
                addr,
                value,
                cache_hit: false,
            },
        };
        self.append_event(event)
    }

    /// Log a memory write operation.
    ///
    /// Records a write to the given address with the written value.
    ///
    /// # Arguments
    ///
    /// * `thread_id` - The thread performing the write.
    /// * `addr` - The memory address being written.
    /// * `value` - The value being written.
    /// * `buffered` - Whether the write is initially buffered (not immediately visible).
    ///
    /// # Errors
    ///
    /// Returns `TracingError::BufferFull` if the backend is at capacity.
    /// Returns `TracingError::InvalidThreadId` if thread_id is out of bounds.
    pub fn log_write(
        &mut self,
        thread_id: ThreadId,
        addr: Address,
        value: u64,
        buffered: bool,
    ) -> Result<(), TracingError> {
        let meta = self.next_meta(thread_id)?;
        let event = SimulationEvent::Memory {
            meta,
            operation: MemoryOperation::Write {
                addr,
                value,
                buffered,
            },
        };
        self.append_event(event)
    }

    /// Log a memory barrier / fence operation.
    ///
    /// Records a fence with the specified semantics (acquire, release, or seq_cst).
    ///
    /// # Arguments
    ///
    /// * `thread_id` - The thread executing the fence.
    /// * `fence_type` - The type of fence (Acquire, Release, or SeqCst).
    ///
    /// # Errors
    ///
    /// Returns `TracingError::BufferFull` if the backend is at capacity.
    /// Returns `TracingError::InvalidThreadId` if thread_id is out of bounds.
    pub fn log_fence(
        &mut self,
        thread_id: ThreadId,
        fence_type: FenceType,
    ) -> Result<(), TracingError> {
        let meta = self.next_meta(thread_id)?;
        let event = SimulationEvent::Memory {
            meta,
            operation: MemoryOperation::Fence { fence_type },
        };
        self.append_event(event)
    }

    /// Log a mutex lock acquisition.
    ///
    /// Records the acquisition of a mutex, establishing a happens-before edge
    /// to any previous unlock of the same mutex.
    ///
    /// # Arguments
    ///
    /// * `thread_id` - The thread acquiring the lock.
    /// * `lock_id` - A unique identifier for the mutex.
    ///
    /// # Errors
    ///
    /// Returns `TracingError::BufferFull` if the backend is at capacity.
    /// Returns `TracingError::InvalidThreadId` if thread_id is out of bounds.
    pub fn log_mutex_lock(
        &mut self,
        thread_id: ThreadId,
        lock_id: u64,
    ) -> Result<(), TracingError> {
        let meta = self.next_meta(thread_id)?;
        let event = SimulationEvent::Synchronization {
            meta,
            sync_event: SyncEvent::MutexLock { lock_id },
        };
        self.append_event(event)
    }

    /// Log a mutex lock release.
    ///
    /// Records the release of a mutex, establishing a happens-before edge
    /// to any subsequent lock of the same mutex.
    ///
    /// # Arguments
    ///
    /// * `thread_id` - The thread releasing the lock.
    /// * `lock_id` - A unique identifier for the mutex.
    ///
    /// # Errors
    ///
    /// Returns `TracingError::BufferFull` if the backend is at capacity.
    /// Returns `TracingError::InvalidThreadId` if thread_id is out of bounds.
    pub fn log_mutex_unlock(
        &mut self,
        thread_id: ThreadId,
        lock_id: u64,
    ) -> Result<(), TracingError> {
        let meta = self.next_meta(thread_id)?;
        let event = SimulationEvent::Synchronization {
            meta,
            sync_event: SyncEvent::MutexUnlock { lock_id },
        };
        self.append_event(event)
    }

    /// Log a condition variable wait.
    ///
    /// Records the thread blocking on a condition variable.
    ///
    /// # Arguments
    ///
    /// * `thread_id` - The thread waiting on the condition variable.
    /// * `cv_id` - A unique identifier for the condition variable.
    ///
    /// # Errors
    ///
    /// Returns `TracingError::BufferFull` if the backend is at capacity.
    /// Returns `TracingError::InvalidThreadId` if thread_id is out of bounds.
    pub fn log_cond_var_wait(
        &mut self,
        thread_id: ThreadId,
        cv_id: u64,
    ) -> Result<(), TracingError> {
        let meta = self.next_meta(thread_id)?;
        let event = SimulationEvent::Synchronization {
            meta,
            sync_event: SyncEvent::CondVarWait { cv_id },
        };
        self.append_event(event)
    }

    /// Log a condition variable signal.
    ///
    /// Records a thread signaling a condition variable.
    ///
    /// # Arguments
    ///
    /// * `thread_id` - The thread signaling the condition variable.
    /// * `cv_id` - A unique identifier for the condition variable.
    ///
    /// # Errors
    ///
    /// Returns `TracingError::BufferFull` if the backend is at capacity.
    /// Returns `TracingError::InvalidThreadId` if thread_id is out of bounds.
    pub fn log_cond_var_signal(
        &mut self,
        thread_id: ThreadId,
        cv_id: u64,
    ) -> Result<(), TracingError> {
        let meta = self.next_meta(thread_id)?;
        let event = SimulationEvent::Synchronization {
            meta,
            sync_event: SyncEvent::CondVarSignal { cv_id },
        };
        self.append_event(event)
    }

    /// Log a thread spawn event.
    ///
    /// Records the creation of a new thread, establishing a parent-to-child
    /// happens-before edge.
    ///
    /// # Arguments
    ///
    /// * `parent_id` - The thread creating the new thread.
    /// * `child_id` - The ID of the newly created thread.
    ///
    /// # Errors
    ///
    /// Returns `TracingError::BufferFull` if the backend is at capacity.
    /// Returns `TracingError::InvalidThreadId` if either thread ID is out of bounds.
    pub fn log_thread_spawn(
        &mut self,
        parent_id: ThreadId,
        child_id: ThreadId,
    ) -> Result<(), TracingError> {
        let meta = self.next_meta(parent_id)?;

        // Synchronize the child thread's clock with the parent's
        let child_idx = child_id.as_index();
        if child_idx < MAX_THREADS {
            self.thread_states[child_idx].sync_with(meta.timestamp);
        }

        let event = SimulationEvent::ThreadSpawn { meta, child_id };
        self.append_event(event)
    }

    /// Log a thread join event.
    ///
    /// Records the parent thread joining on a child thread, establishing
    /// a child-to-parent happens-before edge.
    ///
    /// # Arguments
    ///
    /// * `parent_id` - The thread performing the join.
    /// * `child_id` - The ID of the thread being joined.
    ///
    /// # Errors
    ///
    /// Returns `TracingError::BufferFull` if the backend is at capacity.
    /// Returns `TracingError::InvalidThreadId` if either thread ID is out of bounds.
    pub fn log_thread_join(
        &mut self,
        parent_id: ThreadId,
        child_id: ThreadId,
    ) -> Result<(), TracingError> {
        let child_idx = child_id.as_index();
        let parent_idx = parent_id.as_index();

        if child_idx < MAX_THREADS && parent_idx < MAX_THREADS {
            let child_ts = self.thread_states[child_idx].last_timestamp;
            let parent_state = &mut self.thread_states[parent_idx];

            if parent_state.last_timestamp < child_ts {
                parent_state.last_timestamp = child_ts;
            }
        }
        let meta = self.next_meta(parent_id)?;

        let event = SimulationEvent::ThreadJoin { meta, child_id };
        self.append_event(event)
    }

    /// Get the current number of recorded events.
    #[inline(always)]
    pub fn event_count(&self) -> usize {
        self.backend.event_count()
    }

    /// Get the current global Lamport timestamp.
    #[inline(always)]
    pub fn global_timestamp(&self) -> LamportTimestamp {
        self.backend.global_timestamp()
    }

    /// Get a reference to an event at the specified index.
    #[inline]
    pub fn get_event(&self, index: usize) -> Option<SimulationEvent> {
        self.backend.get_event(index)
    }

    /// Get all recorded events as a slice.
    #[inline]
    pub fn get_all_events(&self) -> &[SimulationEvent] {
        self.backend.get_all_events()
    }

    /// Clear all recorded events, retaining capacity.
    #[inline]
    pub fn clear(&mut self) {
        self.backend.clear();
        self.thread_states = [ThreadState::new(); MAX_THREADS];
    }

    /// Verify that the recorded trace satisfies causality invariants.
    #[inline]
    pub fn verify_causality(&self) -> Result<(), TracingError> {
        self.backend.verify_causality()
    }

    /// Get a reference to the backend (for advanced use cases).
    #[inline]
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Get a mutable reference to the backend (for advanced use cases).
    #[inline]
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }
}

impl<B: TracerBackend> fmt::Debug for TraceEngine<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TraceEngine")
            .field("event_count", &self.event_count())
            .field("global_timestamp", &self.global_timestamp())
            .field("backend", &self.backend)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::tracing::production::ProductionBackend;

    #[test]
    fn test_engine_basic_event_recording() {
        let backend = ProductionBackend::new(100);
        let mut tracer = TraceEngine::new(backend, TraceEngineConfig::default());

        let tid = ThreadId::new(0);
        assert!(tracer.log_read(tid, Address::new(0x1000), 42).is_ok());
        assert_eq!(tracer.event_count(), 1);
    }

    #[test]
    fn test_engine_lamport_clock_increment() {
        let backend = ProductionBackend::new(100);
        let mut tracer = TraceEngine::new(backend, TraceEngineConfig::default());

        let tid = ThreadId::new(0);
        tracer.log_read(tid, Address::new(0x1000), 42).unwrap();
        tracer
            .log_write(tid, Address::new(0x2000), 100, false)
            .unwrap();
        tracer.log_fence(tid, FenceType::SeqCst).unwrap();

        assert_eq!(tracer.event_count(), 3);
        assert_eq!(tracer.global_timestamp().0, 3);
    }

    #[test]
    fn test_engine_mutex_operations() {
        let backend = ProductionBackend::new(100);
        let mut tracer = TraceEngine::new(backend, TraceEngineConfig::default());

        let tid = ThreadId::new(0);
        assert!(tracer.log_mutex_lock(tid, 1).is_ok());
        assert!(tracer.log_read(tid, Address::new(0x1000), 42).is_ok());
        assert!(tracer.log_mutex_unlock(tid, 1).is_ok());

        assert_eq!(tracer.event_count(), 3);
    }

    #[test]
    fn test_engine_thread_operations() {
        let backend = ProductionBackend::new(100);
        let mut tracer = TraceEngine::new(backend, TraceEngineConfig::default());

        let parent = ThreadId::new(0);
        let child = ThreadId::new(1);

        assert!(tracer.log_thread_spawn(parent, child).is_ok());
        assert!(tracer.log_read(child, Address::new(0x1000), 42).is_ok());
        assert!(tracer.log_thread_join(parent, child).is_ok());

        assert_eq!(tracer.event_count(), 3);
    }

    #[test]
    fn test_engine_causality_validation_enabled() {
        let backend = ProductionBackend::new(100);
        let config = TraceEngineConfig {
            validate_causality: true,
        };
        let mut tracer = TraceEngine::new(backend, config);

        let tid = ThreadId::new(0);
        tracer.log_read(tid, Address::new(0x1000), 42).unwrap();

        // Try to manually create an event with a regressed timestamp
        let meta = EventMetadata::new(LamportTimestamp(0), tid, 1);
        let event = SimulationEvent::Memory {
            meta,
            operation: MemoryOperation::Fence {
                fence_type: FenceType::SeqCst,
            },
        };

        let result = tracer.append_event(event);
        assert!(matches!(
            result,
            Err(TracingError::CausalityViolation { .. })
        ));
    }

    #[test]
    fn test_engine_cond_var_operations() {
        let backend = ProductionBackend::new(100);
        let mut tracer = TraceEngine::new(backend, TraceEngineConfig::default());

        let tid = ThreadId::new(0);
        assert!(tracer.log_cond_var_wait(tid, 1).is_ok());
        assert!(tracer.log_cond_var_signal(tid, 1).is_ok());

        assert_eq!(tracer.event_count(), 2);
    }
}
