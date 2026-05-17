// SPDX-License-Identifier: Apache-2.0
//! Production backend for high-performance event recording.
//!
//! This module provides the `ProductionBackend`, optimized for recording
//! millions of events with minimal overhead. It uses heap-allocated storage
//! and achieves near-zero latency appends through efficient memory layout.

use super::traits::{TracerBackend, TracingError};
use super::types::{LamportTimestamp, SimulationEvent, MAX_THREADS};

/// High-performance event storage backend using heap-allocated Vec.
///
/// The ProductionBackend is designed for real-world simulation runs that may
/// record millions of events. It prioritizes throughput and latency over
/// space efficiency, using pre-allocated capacity to avoid dynamic resizing
/// during normal operation.
///
/// # Memory Layout
///
/// Events are stored in a single Vec, with each event occupying exactly
/// 64 bytes (cache-line aligned). This minimizes cache conflicts and improves
/// sequential read throughput for offline analysis.
///
/// # Performance Characteristics
///
/// - **Append**: O(1) amortized. Single Vec push with pre-allocated capacity.
/// - **Get Event**: O(1). Direct indexing into the Vec.
/// - **Clear**: O(1). Resets the vector length without deallocation.
/// - **Verify Causality**: O(n * MAX_THREADS). Single pass through all events.
///
/// # Example
///
/// ```ignore
/// use laplace_core::domain::tracing::{ProductionBackend, TraceEngine};
///
/// let backend = ProductionBackend::with_capacity(100_000);
/// let mut tracer = TraceEngine::new(backend, Default::default());
/// ```
#[derive(Debug)]
pub struct ProductionBackend {
    /// Event storage (pre-allocated Vec for efficiency).
    events: Vec<SimulationEvent>,
    /// Global Lamport timestamp (max of all event timestamps).
    global_timestamp: LamportTimestamp,
}

impl ProductionBackend {
    /// Create a new ProductionBackend with specified capacity.
    ///
    /// The backend will pre-allocate capacity for the specified number of events,
    /// minimizing allocations during recording. Capacity can grow beyond this
    /// limit if needed, but this is typically avoided in production scenarios.
    ///
    /// # Arguments
    ///
    /// * `capacity` - The initial capacity (number of events to pre-allocate).
    ///
    /// # Example
    ///
    /// ```ignore
    /// let backend = ProductionBackend::new(100_000);
    /// ```
    #[inline]
    pub fn new(capacity: usize) -> Self {
        Self {
            events: Vec::with_capacity(capacity),
            global_timestamp: LamportTimestamp::ZERO,
        }
    }

    /// Convenience constructor with explicit name.
    ///
    /// This is equivalent to `new()` and is provided for readability.
    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::new(capacity)
    }

    /// Get the underlying Vec of events (for advanced use cases).
    ///
    /// This method allows direct access to the event storage for specialized
    /// analysis or serialization scenarios.
    #[inline]
    pub fn events_ref(&self) -> &[SimulationEvent] {
        &self.events
    }

    /// Get mutable reference to the events Vec.
    ///
    /// # Safety
    ///
    /// Direct mutation of events should be avoided. Use only for bulk initialization
    /// or specialized analysis workflows.
    #[inline]
    pub fn events_mut(&mut self) -> &mut Vec<SimulationEvent> {
        &mut self.events
    }
}

impl TracerBackend for ProductionBackend {
    /// Return the current capacity of the backend.
    #[inline(always)]
    fn max_events(&self) -> usize {
        self.events.capacity()
    }

    /// Append an event to the trace.
    ///
    /// This operation is O(1) amortized, with the backend automatically
    /// updating the global timestamp to maintain the maximum invariant.
    ///
    /// # Errors
    ///
    /// Returns `TracingError::BufferFull` if the buffer has reached capacity
    /// and memory allocation fails. In typical scenarios, this should not occur
    /// if capacity was chosen appropriately.
    #[inline]
    fn append_event(&mut self, event: SimulationEvent) -> Result<(), TracingError> {
        let ts = event.timestamp();
        self.events.push(event);
        self.update_global_timestamp(ts);
        Ok(())
    }

    /// Get the event at the specified index.
    ///
    /// Returns a copy of the event if it exists, or None if the index is out of bounds.
    /// Since `SimulationEvent` is a small copy type (64 bytes), this is efficient.
    #[inline]
    fn get_event(&self, index: usize) -> Option<SimulationEvent> {
        self.events.get(index).copied()
    }

    /// Get all events as a slice.
    ///
    /// Returns a reference to the underlying event slice, enabling efficient
    /// bulk analysis and iteration.
    #[inline]
    fn get_all_events(&self) -> &[SimulationEvent] {
        &self.events
    }

    /// Get the current number of recorded events.
    #[inline(always)]
    fn event_count(&self) -> usize {
        self.events.len()
    }

    /// Get the current global Lamport timestamp.
    #[inline(always)]
    fn global_timestamp(&self) -> LamportTimestamp {
        self.global_timestamp
    }

    /// Update the global timestamp to track the maximum seen so far.
    ///
    /// This maintains the invariant that global_timestamp is always the
    /// maximum of all event timestamps.
    #[inline(always)]
    fn update_global_timestamp(&mut self, ts: LamportTimestamp) {
        if ts > self.global_timestamp {
            self.global_timestamp = ts;
        }
    }

    /// Clear all events while retaining capacity.
    ///
    /// This efficiently resets the trace while keeping the pre-allocated
    /// buffer intact for reuse.
    #[inline]
    fn clear(&mut self) {
        self.events.clear();
        self.global_timestamp = LamportTimestamp::ZERO;
    }

    /// Verify that the recorded trace satisfies causality invariants.
    ///
    /// This method checks:
    /// - Within each thread, timestamps are monotonically increasing.
    /// - Global timestamp matches the maximum of all event timestamps.
    ///
    /// # Returns
    ///
    /// - `Ok(())` if all invariants are satisfied.
    /// - `Err(TracingError::CausalityViolation { ... })` if a thread has a timestamp regression.
    fn verify_causality(&self) -> Result<(), TracingError> {
        let mut thread_timestamps = [LamportTimestamp::ZERO; MAX_THREADS];
        let mut computed_global = LamportTimestamp::ZERO;

        for event in &self.events {
            let meta = event.metadata();
            let thread_idx = meta.thread_id.as_index();

            let last_ts = thread_timestamps[thread_idx];
            if meta.timestamp <= last_ts && last_ts.0 != 0 {
                return Err(TracingError::CausalityViolation {
                    expected_min: last_ts,
                    received: meta.timestamp,
                });
            }

            thread_timestamps[thread_idx] = meta.timestamp;

            if meta.timestamp > computed_global {
                computed_global = meta.timestamp;
            }
        }

        if computed_global != self.global_timestamp && !self.events.is_empty() {
            return Err(TracingError::CausalityViolation {
                expected_min: computed_global,
                received: self.global_timestamp,
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::tracing::types::{EventMetadata, MemoryOperation, ThreadId};

    #[test]
    fn test_production_backend_append() {
        let mut backend = ProductionBackend::new(10);

        let meta = EventMetadata::new(LamportTimestamp(1), ThreadId::new(0), 0);
        let event = SimulationEvent::Memory {
            meta,
            operation: MemoryOperation::Fence {
                fence_type: super::super::types::FenceType::SeqCst,
            },
        };

        assert!(backend.append_event(event).is_ok());
        assert_eq!(backend.event_count(), 1);
        assert_eq!(backend.global_timestamp().0, 1);
    }

    #[test]
    fn test_production_backend_get_event() {
        let mut backend = ProductionBackend::new(10);

        let meta = EventMetadata::new(LamportTimestamp(42), ThreadId::new(1), 5);
        let event = SimulationEvent::Memory {
            meta,
            operation: MemoryOperation::Fence {
                fence_type: super::super::types::FenceType::Release,
            },
        };

        backend.append_event(event).unwrap();

        let retrieved = backend.get_event(0);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().timestamp().0, 42);
    }

    #[test]
    fn test_production_backend_clear() {
        let mut backend = ProductionBackend::new(10);

        let meta = EventMetadata::new(LamportTimestamp(1), ThreadId::new(0), 0);
        let event = SimulationEvent::Memory {
            meta,
            operation: MemoryOperation::Fence {
                fence_type: super::super::types::FenceType::Acquire,
            },
        };

        backend.append_event(event).unwrap();
        assert_eq!(backend.event_count(), 1);

        backend.clear();
        assert_eq!(backend.event_count(), 0);
        assert_eq!(backend.global_timestamp(), LamportTimestamp::ZERO);
    }

    #[test]
    fn test_production_backend_global_timestamp() {
        let mut backend = ProductionBackend::new(10);

        let meta1 = EventMetadata::new(LamportTimestamp(5), ThreadId::new(0), 0);
        let meta2 = EventMetadata::new(LamportTimestamp(3), ThreadId::new(1), 0);
        let meta3 = EventMetadata::new(LamportTimestamp(10), ThreadId::new(0), 1);

        let event1 = SimulationEvent::Memory {
            meta: meta1,
            operation: MemoryOperation::Fence {
                fence_type: super::super::types::FenceType::SeqCst,
            },
        };
        let event2 = SimulationEvent::Memory {
            meta: meta2,
            operation: MemoryOperation::Fence {
                fence_type: super::super::types::FenceType::SeqCst,
            },
        };
        let event3 = SimulationEvent::Memory {
            meta: meta3,
            operation: MemoryOperation::Fence {
                fence_type: super::super::types::FenceType::SeqCst,
            },
        };

        backend.append_event(event1).unwrap();
        assert_eq!(backend.global_timestamp().0, 5);

        backend.append_event(event2).unwrap();
        assert_eq!(backend.global_timestamp().0, 5);

        backend.append_event(event3).unwrap();
        assert_eq!(backend.global_timestamp().0, 10);
    }

    #[test]
    fn test_production_backend_verify_causality_valid() {
        let mut backend = ProductionBackend::new(10);

        let meta1 = EventMetadata::new(LamportTimestamp(1), ThreadId::new(0), 0);
        let meta2 = EventMetadata::new(LamportTimestamp(2), ThreadId::new(0), 1);
        let meta3 = EventMetadata::new(LamportTimestamp(3), ThreadId::new(1), 0);

        let event1 = SimulationEvent::Memory {
            meta: meta1,
            operation: MemoryOperation::Fence {
                fence_type: super::super::types::FenceType::SeqCst,
            },
        };
        let event2 = SimulationEvent::Memory {
            meta: meta2,
            operation: MemoryOperation::Fence {
                fence_type: super::super::types::FenceType::SeqCst,
            },
        };
        let event3 = SimulationEvent::Memory {
            meta: meta3,
            operation: MemoryOperation::Fence {
                fence_type: super::super::types::FenceType::SeqCst,
            },
        };

        backend.append_event(event1).unwrap();
        backend.append_event(event2).unwrap();
        backend.append_event(event3).unwrap();

        assert!(backend.verify_causality().is_ok());
    }

    #[test]
    fn test_production_backend_verify_causality_violation() {
        let mut backend = ProductionBackend::new(10);

        let meta1 = EventMetadata::new(LamportTimestamp(5), ThreadId::new(0), 0);
        let meta2 = EventMetadata::new(LamportTimestamp(3), ThreadId::new(0), 1);

        let event1 = SimulationEvent::Memory {
            meta: meta1,
            operation: MemoryOperation::Fence {
                fence_type: super::super::types::FenceType::SeqCst,
            },
        };
        let event2 = SimulationEvent::Memory {
            meta: meta2,
            operation: MemoryOperation::Fence {
                fence_type: super::super::types::FenceType::SeqCst,
            },
        };

        backend.append_event(event1).unwrap();
        backend.append_event(event2).unwrap();

        assert!(matches!(
            backend.verify_causality(),
            Err(TracingError::CausalityViolation { .. })
        ));
    }
}
