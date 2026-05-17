//! Verification backend for Kani-compatible formal verification.
//!
//! This module provides the `VerificationBackend`, optimized for bounded model
//! checking with the Kani formal verification tool. It uses a fixed-size array
//! to keep the solver's state space tractable and explicitly tracks initialization
//! state using `Option` for Kani-provable invariant checking.
//!
//! **Note**: This module is only available when `feature = "twin"` is enabled.
//! It is not intended for production use.

#![cfg(feature = "twin")]

use super::traits::{TracerBackend, TracingError};
use super::types::{LamportTimestamp, SimulationEvent, MAX_THREADS};

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Fixed-size capacity for the verification backend.
///
/// This is deliberately small (64 events) to keep Kani's solver runtime
/// manageable while still covering interesting multi-threaded scenarios.
/// Increase this only if verification time is acceptable.
pub const VERIFICATION_MAX_EVENTS: usize = 64;

/// Fixed-capacity event storage backend for formal verification.
///
/// The VerificationBackend is designed specifically for use with Kani formal
/// verification tool. It uses a fixed-size array of `Option<SimulationEvent>`
/// to make initialization state explicit to the solver, enabling bounded
/// model checking of causality invariants.
///
/// # Memory Layout
///
/// - Events: `[Option<SimulationEvent>; 64]` (4096 bytes, stack-allocated)
/// - Global timestamp: 8 bytes
/// - Total: ~4 KB per instance (stack-friendly for verification)
///
/// # Design Rationale
///
/// 1. **Option-based Storage**: Using `Option` instead of `MaybeUninit` makes
///    the initialization state explicit to Kani, enabling the solver to reason
///    about which slots are filled.
///
/// 2. **Fixed Capacity**: Keeps the state space bounded, enabling complete
///    verification without exhaustive exploration becoming infeasible.
///
/// 3. **Stack Allocation**: No heap allocation means no potential issues with
///    dynamic memory in verification harnesses.
///
/// # Example (Feature-Gated)
///
/// ```ignore
/// #[cfg(feature = "twin")]
/// {
///     use laplace_core::domain::tracing::{VerificationBackend, TraceEngine};
///
///     let backend = VerificationBackend::new();
///     let mut tracer = TraceEngine::new(backend, Default::default());
/// }
/// ```
#[derive(Debug)]
pub struct VerificationBackend {
    /// Event storage using Option to track initialization state.
    /// None indicates the slot is uninitialized; Some(event) indicates an appended event.
    events: [Option<SimulationEvent>; VERIFICATION_MAX_EVENTS],
    /// Current number of events recorded.
    event_count: usize,
    /// Global Lamport timestamp (max of all event timestamps).
    global_timestamp: LamportTimestamp,
}

impl VerificationBackend {
    /// Create a new VerificationBackend with all slots initialized to None.
    ///
    /// This is a const function, suitable for use in const contexts and
    /// during formal verification harness setup.
    pub const fn new() -> Self {
        Self {
            events: [None; VERIFICATION_MAX_EVENTS],
            event_count: 0,
            global_timestamp: LamportTimestamp::ZERO,
        }
    }
}

impl Default for VerificationBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl TracerBackend for VerificationBackend {
    /// Return the fixed maximum capacity of this backend.
    #[inline(always)]
    fn max_events(&self) -> usize {
        VERIFICATION_MAX_EVENTS
    }

    /// Append an event to the trace.
    ///
    /// # Errors
    ///
    /// Returns `TracingError::BufferFull` if the event_count has reached
    /// `VERIFICATION_MAX_EVENTS`.
    #[inline]
    fn append_event(&mut self, event: SimulationEvent) -> Result<(), TracingError> {
        if self.event_count >= VERIFICATION_MAX_EVENTS {
            return Err(TracingError::BufferFull);
        }

        let ts = event.timestamp();

        // SAFETY: event_count is bounds-checked above
        self.events[self.event_count] = Some(event);
        self.event_count += 1;

        self.update_global_timestamp(ts);

        Ok(())
    }

    /// Get the event at the specified index.
    ///
    /// Returns a copy of the event if the slot contains Some(event), or None
    /// if the index is out of bounds or the slot is uninitialized.
    #[inline]
    fn get_event(&self, index: usize) -> Option<SimulationEvent> {
        if index < self.event_count {
            self.events[index]
        } else {
            None
        }
    }

    /// Get all events as a slice.
    ///
    /// # Note
    ///
    /// For VerificationBackend, this method returns an empty slice because
    /// the underlying storage is `[Option<SimulationEvent>; N]`, not
    /// `[SimulationEvent]`. During formal verification, prefer using
    /// `get_event()` for individual event access or iterating via a manual loop.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Preferred for verification:
    /// for i in 0..tracer.event_count() {
    ///     if let Some(event) = tracer.get_event(i) {
    ///         // process event
    ///     }
    /// }
    /// ```
    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(
            layer = "20_Core_Tracing",
            link = "LEP-0005-laplace-core-tracing_causality"
        )
    )]
    #[inline]
    fn get_all_events(&self) -> &[SimulationEvent] {
        // Cannot return a slice of Option<T>, so return empty slice.
        // This is acceptable because Kani verification typically accesses
        // events individually via get_event().
        &[]
    }

    /// Get the current number of recorded events.
    #[inline(always)]
    fn event_count(&self) -> usize {
        self.event_count
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
    /// Resets event_count to zero and clears the global timestamp.
    /// The slots are not explicitly zeroed (just marked as unused by event_count).
    #[inline]
    fn clear(&mut self) {
        self.event_count = 0;
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
    /// - `Err(TracingError::CausalityViolation { ... })` if a thread has timestamp regression.
    fn verify_causality(&self) -> Result<(), TracingError> {
        let mut thread_timestamps = [LamportTimestamp::ZERO; MAX_THREADS];
        let mut computed_global = LamportTimestamp::ZERO;

        for i in 0..self.event_count {
            if let Some(event) = self.events[i] {
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
        }

        if computed_global != self.global_timestamp && self.event_count > 0 {
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
    use crate::domain::tracing::types::{EventMetadata, FenceType, MemoryOperation, ThreadId};

    #[test]
    fn test_verification_backend_new() {
        let backend = VerificationBackend::new();
        assert_eq!(backend.event_count(), 0);
        assert_eq!(backend.global_timestamp(), LamportTimestamp::ZERO);
    }

    #[test]
    fn test_verification_backend_append() {
        let mut backend = VerificationBackend::new();

        let meta = EventMetadata::new(LamportTimestamp(1), ThreadId::new(0), 0);
        let event = SimulationEvent::Memory {
            meta,
            operation: MemoryOperation::Fence {
                fence_type: FenceType::SeqCst,
            },
        };

        assert!(backend.append_event(event).is_ok());
        assert_eq!(backend.event_count(), 1);
        assert_eq!(backend.global_timestamp().0, 1);
    }

    #[test]
    fn test_verification_backend_buffer_full() {
        let mut backend = VerificationBackend::new();

        for i in 0..VERIFICATION_MAX_EVENTS {
            let meta = EventMetadata::new(LamportTimestamp(i as u64), ThreadId::new(0), i as u64);
            let event = SimulationEvent::Memory {
                meta,
                operation: MemoryOperation::Fence {
                    fence_type: FenceType::SeqCst,
                },
            };
            assert!(backend.append_event(event).is_ok());
        }

        // Next append should fail
        let meta = EventMetadata::new(
            LamportTimestamp(VERIFICATION_MAX_EVENTS as u64),
            ThreadId::new(0),
            VERIFICATION_MAX_EVENTS as u64,
        );
        let event = SimulationEvent::Memory {
            meta,
            operation: MemoryOperation::Fence {
                fence_type: FenceType::SeqCst,
            },
        };

        assert!(matches!(
            backend.append_event(event),
            Err(TracingError::BufferFull)
        ));
    }

    #[test]
    fn test_verification_backend_get_event() {
        let mut backend = VerificationBackend::new();

        let meta = EventMetadata::new(LamportTimestamp(42), ThreadId::new(1), 5);
        let event = SimulationEvent::Memory {
            meta,
            operation: MemoryOperation::Fence {
                fence_type: FenceType::Release,
            },
        };

        backend.append_event(event).unwrap();

        let retrieved = backend.get_event(0);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().timestamp().0, 42);

        assert!(backend.get_event(1).is_none());
    }

    #[test]
    fn test_verification_backend_clear() {
        let mut backend = VerificationBackend::new();

        let meta = EventMetadata::new(LamportTimestamp(1), ThreadId::new(0), 0);
        let event = SimulationEvent::Memory {
            meta,
            operation: MemoryOperation::Fence {
                fence_type: FenceType::Acquire,
            },
        };

        backend.append_event(event).unwrap();
        assert_eq!(backend.event_count(), 1);

        backend.clear();
        assert_eq!(backend.event_count(), 0);
        assert_eq!(backend.global_timestamp(), LamportTimestamp::ZERO);
    }

    #[test]
    fn test_verification_backend_verify_causality_valid() {
        let mut backend = VerificationBackend::new();

        let meta1 = EventMetadata::new(LamportTimestamp(1), ThreadId::new(0), 0);
        let meta2 = EventMetadata::new(LamportTimestamp(2), ThreadId::new(0), 1);

        let event1 = SimulationEvent::Memory {
            meta: meta1,
            operation: MemoryOperation::Fence {
                fence_type: FenceType::SeqCst,
            },
        };
        let event2 = SimulationEvent::Memory {
            meta: meta2,
            operation: MemoryOperation::Fence {
                fence_type: FenceType::SeqCst,
            },
        };

        backend.append_event(event1).unwrap();
        backend.append_event(event2).unwrap();

        assert!(backend.verify_causality().is_ok());
    }

    #[test]
    fn test_verification_backend_verify_causality_violation() {
        let mut backend = VerificationBackend::new();

        let meta1 = EventMetadata::new(LamportTimestamp(5), ThreadId::new(0), 0);
        let meta2 = EventMetadata::new(LamportTimestamp(3), ThreadId::new(0), 1);

        let event1 = SimulationEvent::Memory {
            meta: meta1,
            operation: MemoryOperation::Fence {
                fence_type: FenceType::SeqCst,
            },
        };
        let event2 = SimulationEvent::Memory {
            meta: meta2,
            operation: MemoryOperation::Fence {
                fence_type: FenceType::SeqCst,
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
