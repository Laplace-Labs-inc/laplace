//! Verification-grade clock backend implementation with fixed-size array storage.
//!
//! This module provides the `VerificationBackend`, designed specifically for formal
//! verification scenarios with Kani and testing frameworks. It uses a fixed-size array
//! with Option slots to eliminate dynamic allocation and state-space-exploding operations.
//!
//! # Zero-Cost Razor Strategy for Symbolic Execution
//!
//! Instead of a Vec with insert/remove (which shift elements and create combinatorial
//! state explosions), this backend uses a fixed array of Option slots. This reduces
//! Kani's state space from combinatorial permutations to linear scans.
//!
//! **Feature Gating**: This module is only compiled when the `twin` feature is enabled,
//! as it is intended for test and verification contexts, not production deployments.

#![cfg(feature = "twin")]

use super::types::{LamportClock, ScheduledEvent, VirtualTimeNs};
use super::ClockBackend;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Maximum number of events that can be scheduled in the verification backend.
///
/// # Capacity Strategy
///
/// - **Kani Builds** (`#[cfg(kani)]`): Set to 4 to keep symbolic execution tractable.
///   The fixed array avoids shift operations, so 4 events allow meaningful formal
///   verification of all core invariants without OOM.
///
/// - **Integration Tests** (`#[cfg(not(kani))]`): Set to 64 for realistic test
///   scenarios. Stack allocation keeps memory usage low while allowing moderate
///   event sequences.
#[cfg(kani)]
const CAPACITY: usize = 4;

#[cfg(not(kani))]
const CAPACITY: usize = 64;

/// Verification-grade clock backend using fixed-size array storage.
///
/// The `VerificationBackend` is a verification-focused implementation of `ClockBackend`
/// that trades some runtime performance for symbolic execution tractability.
///
/// # Design Rationale
///
/// Uses `[Option<ScheduledEvent>; CAPACITY]` instead of Vec because:
/// - **No dynamic allocation**: Entire state fits on the stack
/// - **No element shifting**: Operations don't move memory around, reducing state space
/// - **Linear scans only**: `pop_event` finds the minimum by iteration, not binary search
/// - **Bounded complexity**: Kani can explore all branches without combinatorial explosion
///
/// ## Why This Matters for Kani
///
/// Vector operations like `insert` and `remove` are devastating for symbolic execution:
/// - Inserting at position i shifts n-i elements
/// - Each shift creates a distinct state transition
/// - All possible permutations become reachable states
/// - State space grows factorially, causing OOM
///
/// With fixed arrays, insertion just sets an Option slot:
/// - O(1) to find an empty slot
/// - O(1) to set it to Some(event)
/// - State space remains bounded and linear
///
/// # Storage Invariant
///
/// The queue is stored as a sparse array where:
/// - `events[i] = None` means slot i is empty
/// - `events[i] = Some(e)` means slot i contains event e
/// - `len` tracks the number of occupied slots
///
/// Events are **not** stored in sorted order. Instead, `pop_event` scans the entire
/// array to find the minimum element, maintaining temporal correctness at the cost
/// of O(n) per pop.
///
/// # Performance Characteristics
///
/// - **Schedule Event**: O(n) to find an empty slot (typically O(1) amortized)
/// - **Process Event**: O(n²) worst case (scan to find min, then scan again for next)
/// - **Current Time/Lamport**: O(1)
/// - **Memory**: Fixed at ~64 × 32 bytes = 2 KB on the stack
///
/// # Feature Flag
///
/// This implementation is only available when `feature = "twin"` is enabled.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(layer = "20_Core_Time", link = "LEP-0001-laplace-core-time_domain")
)]
#[derive(Debug)]
pub struct VerificationBackend {
    /// Current virtual time in nanoseconds.
    ///
    /// Advances only when events are processed. Starts at zero and is
    /// monotonically non-decreasing.
    current_time: VirtualTimeNs,

    /// Current Lamport logical clock value.
    ///
    /// Incremented on each event for causality tracking. Reset to zero
    /// when the event queue becomes empty (per TLA+ spec).
    lamport: LamportClock,

    /// Fixed-size event storage using Option slots.
    ///
    /// Slots can be empty (None) or contain an event (Some(e)).
    /// No sorting invariant - events are stored in arbitrary order.
    /// The `len` field tracks how many slots are occupied.
    events: [Option<ScheduledEvent>; CAPACITY],

    /// Count of occupied slots in the events array.
    ///
    /// Invariant: `len <= CAPACITY` and equals the number of `Some(_)` in events.
    len: usize,
}

impl VerificationBackend {
    /// Create a new verification clock backend.
    ///
    /// Initializes the backend with zero time, zero Lamport clock, and an
    /// empty event queue (all Option slots are None).
    ///
    /// # Returns
    ///
    /// A new `VerificationBackend` initialized to its starting state.
    pub fn new() -> Self {
        Self {
            current_time: 0,
            lamport: 0,
            events: [const { None }; CAPACITY],
            len: 0,
        }
    }
}

impl Default for VerificationBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl ClockBackend for VerificationBackend {
    fn current_time(&self) -> VirtualTimeNs {
        self.current_time
    }

    fn current_lamport(&self) -> LamportClock {
        self.lamport
    }

    fn set_time(&mut self, time: VirtualTimeNs) {
        self.current_time = time;
    }

    fn increment_lamport(&mut self) -> LamportClock {
        self.lamport = self.lamport.wrapping_add(1);
        self.lamport
    }

    fn reset_lamport(&mut self) {
        self.lamport = 0;
    }

    fn push_event(&mut self, event: ScheduledEvent) -> bool {
        // Check capacity bound
        if self.len >= CAPACITY {
            return false;
        }

        // Find the first empty slot and insert the event
        for slot in self.events.iter_mut() {
            if slot.is_none() {
                *slot = Some(event);
                self.len += 1;
                return true;
            }
        }

        // Should never reach here if len < CAPACITY and invariants hold
        false
    }

    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(layer = "20_Core_Time", link = "LEP-0001-laplace-core-time_domain")
    )]
    fn pop_event(&mut self) -> Option<ScheduledEvent> {
        if self.len == 0 {
            return None;
        }

        // Find the index of the best event (earliest time, then lowest lamport)
        // Note: ScheduledEvent implements Ord in REVERSE for Max-Heap usage.
        // So `a > b` means `a` is earlier (higher priority) than `b`.
        let mut best_idx: Option<usize> = None;

        for i in 0..CAPACITY {
            if let Some(current_event) = &self.events[i] {
                match best_idx {
                    None => best_idx = Some(i),
                    Some(best) => {
                        // Compare current event with the best found so far
                        // We use `>` because Ord is reversed (Max-Heap style)
                        // A "Greater" event means it has an Earlier timestamp.
                        let best_event = self.events[best].as_ref().unwrap();
                        if current_event > best_event {
                            best_idx = Some(i);
                        }
                    }
                }
            }
        }

        // Extract and return the best event
        if let Some(idx) = best_idx {
            // take() leaves None in the slot and returns the event
            let event = self.events[idx].take();
            self.len -= 1;

            // Reset Lamport when queue becomes empty (per TLA+ spec)
            if self.len == 0 {
                self.reset_lamport();
            }

            event
        } else {
            None
        }
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn queue_len(&self) -> usize {
        self.len
    }

    fn reset(&mut self) {
        self.current_time = 0;
        self.lamport = 0;
        self.events = [const { None }; CAPACITY];
        self.len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::memory::CoreId;
    use crate::domain::time::EventPayload;

    #[test]
    fn test_verification_backend_creation() {
        let backend = VerificationBackend::new();
        assert_eq!(backend.current_time(), 0);
        assert_eq!(backend.current_lamport(), 0);
        assert!(backend.is_empty());
        assert_eq!(backend.queue_len(), 0);
    }

    #[test]
    fn test_verification_backend_push_event() {
        let mut backend = VerificationBackend::new();
        let core0 = CoreId::new(0);

        let event = ScheduledEvent::new(100, 0, 1, EventPayload::MemoryFence { core: core0 });

        assert!(backend.push_event(event));
        assert_eq!(backend.queue_len(), 1);
        assert!(!backend.is_empty());
    }

    #[test]
    fn test_verification_backend_push_event_respects_capacity() {
        let mut backend = VerificationBackend::new();

        // Fill to capacity
        for i in 0..CAPACITY {
            let event = ScheduledEvent::new(i as u64, 0, i as u64, EventPayload::Test(i as u64));
            assert!(
                backend.push_event(event),
                "Push should succeed within capacity"
            );
        }

        // Next push should fail
        let event = ScheduledEvent::new(CAPACITY as u64, 0, CAPACITY as u64, EventPayload::Test(0));
        assert!(
            !backend.push_event(event),
            "Push should fail when at capacity"
        );
        assert_eq!(backend.queue_len(), CAPACITY);
    }

    #[test]
    fn test_verification_backend_pop_event() {
        let mut backend = VerificationBackend::new();

        let event = ScheduledEvent::new(100, 0, 1, EventPayload::Test(0));
        backend.push_event(event.clone());

        let popped = backend.pop_event();
        assert!(popped.is_some());
        assert_eq!(popped.unwrap().event_id, 1);
        assert!(backend.is_empty());
    }

    #[test]
    fn test_verification_backend_event_ordering_by_time() {
        let mut backend = VerificationBackend::new();

        // Add events in reverse time order
        backend.push_event(ScheduledEvent::new(300, 0, 3, EventPayload::Test(0)));
        backend.push_event(ScheduledEvent::new(100, 0, 1, EventPayload::Test(0)));
        backend.push_event(ScheduledEvent::new(200, 0, 2, EventPayload::Test(0)));

        // Should pop in ascending time order
        assert_eq!(backend.pop_event().unwrap().scheduled_at_ns, 100);
        assert_eq!(backend.pop_event().unwrap().scheduled_at_ns, 200);
        assert_eq!(backend.pop_event().unwrap().scheduled_at_ns, 300);
    }

    #[test]
    fn test_verification_backend_event_ordering_by_lamport() {
        let mut backend = VerificationBackend::new();

        // Events at same time, different Lamport values
        backend.push_event(ScheduledEvent::new(100, 3, 1, EventPayload::Test(0)));
        backend.push_event(ScheduledEvent::new(100, 1, 2, EventPayload::Test(0)));
        backend.push_event(ScheduledEvent::new(100, 2, 3, EventPayload::Test(0)));

        // Should pop in ascending Lamport order
        assert_eq!(backend.pop_event().unwrap().lamport, 1);
        assert_eq!(backend.pop_event().unwrap().lamport, 2);
        assert_eq!(backend.pop_event().unwrap().lamport, 3);
    }

    #[test]
    fn test_verification_backend_event_ordering_by_id() {
        let mut backend = VerificationBackend::new();

        // Events at same time and Lamport, different IDs
        backend.push_event(ScheduledEvent::new(100, 1, 3, EventPayload::Test(0)));
        backend.push_event(ScheduledEvent::new(100, 1, 1, EventPayload::Test(0)));
        backend.push_event(ScheduledEvent::new(100, 1, 2, EventPayload::Test(0)));

        // Should pop in ascending ID order
        assert_eq!(backend.pop_event().unwrap().event_id, 1);
        assert_eq!(backend.pop_event().unwrap().event_id, 2);
        assert_eq!(backend.pop_event().unwrap().event_id, 3);
    }

    #[test]
    fn test_verification_backend_time_advance() {
        let mut backend = VerificationBackend::new();

        assert_eq!(backend.current_time(), 0);
        backend.set_time(5000);
        assert_eq!(backend.current_time(), 5000);
    }

    #[test]
    fn test_verification_backend_lamport_increment() {
        let mut backend = VerificationBackend::new();

        assert_eq!(backend.current_lamport(), 0);
        let new_lamport = backend.increment_lamport();
        assert_eq!(new_lamport, 1);
        assert_eq!(backend.current_lamport(), 1);
    }

    #[test]
    fn test_verification_backend_lamport_reset() {
        let mut backend = VerificationBackend::new();

        backend.increment_lamport();
        assert_eq!(backend.current_lamport(), 1);

        backend.reset_lamport();
        assert_eq!(backend.current_lamport(), 0);
    }

    #[test]
    fn test_verification_backend_reset() {
        let mut backend = VerificationBackend::new();
        let core0 = CoreId::new(0);

        backend.set_time(5000);
        backend.increment_lamport();
        backend.push_event(ScheduledEvent::new(
            1000,
            1,
            1,
            EventPayload::MemoryFence { core: core0 },
        ));

        assert_eq!(backend.current_time(), 5000);
        assert_eq!(backend.current_lamport(), 1);
        assert_eq!(backend.queue_len(), 1);

        backend.reset();

        assert_eq!(backend.current_time(), 0);
        assert_eq!(backend.current_lamport(), 0);
        assert_eq!(backend.queue_len(), 0);
    }

    #[test]
    fn test_verification_backend_lamport_reset_on_empty() {
        let mut backend = VerificationBackend::new();

        backend.push_event(ScheduledEvent::new(100, 0, 1, EventPayload::Test(0)));
        backend.push_event(ScheduledEvent::new(200, 0, 2, EventPayload::Test(0)));
        backend.increment_lamport();

        backend.pop_event();
        assert!(!backend.is_empty());
        assert_eq!(backend.current_lamport(), 1);

        backend.pop_event();
        assert!(backend.is_empty());
        assert_eq!(backend.current_lamport(), 0);
    }

    #[test]
    fn test_verification_backend_default() {
        let backend = VerificationBackend::default();
        assert_eq!(backend.current_time(), 0);
        assert_eq!(backend.current_lamport(), 0);
        assert!(backend.is_empty());
    }

    #[test]
    fn test_verification_backend_pop_order_maintained() {
        let mut backend = VerificationBackend::new();

        // Push events in random order
        backend.push_event(ScheduledEvent::new(500, 0, 5, EventPayload::Test(0)));
        backend.push_event(ScheduledEvent::new(100, 0, 1, EventPayload::Test(0)));
        backend.push_event(ScheduledEvent::new(300, 0, 3, EventPayload::Test(0)));
        backend.push_event(ScheduledEvent::new(200, 0, 2, EventPayload::Test(0)));

        // Pop all events and verify they come out in time order
        let mut times = Vec::new();
        while let Some(event) = backend.pop_event() {
            times.push(event.scheduled_at_ns);
        }
        assert_eq!(times, vec![100, 200, 300, 500]);
    }

    #[test]
    fn test_verification_backend_sparse_array_correctness() {
        let mut backend = VerificationBackend::new();

        // Push some events
        backend.push_event(ScheduledEvent::new(10, 0, 1, EventPayload::Test(0)));
        backend.push_event(ScheduledEvent::new(20, 0, 2, EventPayload::Test(0)));
        backend.push_event(ScheduledEvent::new(30, 0, 3, EventPayload::Test(0)));
        assert_eq!(backend.queue_len(), 3);

        // Pop one
        let first = backend.pop_event().unwrap();
        assert_eq!(first.scheduled_at_ns, 10);
        assert_eq!(backend.queue_len(), 2);

        // Push another (should fill a slot, not allocate)
        backend.push_event(ScheduledEvent::new(15, 0, 4, EventPayload::Test(0)));
        assert_eq!(backend.queue_len(), 3);

        // Pop in order
        let second = backend.pop_event().unwrap();
        assert_eq!(second.scheduled_at_ns, 15);

        let third = backend.pop_event().unwrap();
        assert_eq!(third.scheduled_at_ns, 20);

        let fourth = backend.pop_event().unwrap();
        assert_eq!(fourth.scheduled_at_ns, 30);

        assert!(backend.is_empty());
    }

    #[test]
    fn test_verification_backend_capacity_boundary() {
        let mut backend = VerificationBackend::new();

        // Fill exactly to capacity
        for i in 0..CAPACITY {
            let success = backend.push_event(ScheduledEvent::new(
                i as u64,
                0,
                i as u64,
                EventPayload::Test(i as u64),
            ));
            assert!(success, "Push {} should succeed", i);
        }

        // Verify capacity reached
        assert_eq!(backend.queue_len(), CAPACITY);

        // Try to exceed capacity
        let overflow = backend.push_event(ScheduledEvent::new(
            CAPACITY as u64,
            0,
            999,
            EventPayload::Test(0),
        ));
        assert!(!overflow, "Should reject push at capacity");

        // Pop one event
        backend.pop_event();
        assert_eq!(backend.queue_len(), CAPACITY - 1);

        // Now we should be able to push again
        let success = backend.push_event(ScheduledEvent::new(
            CAPACITY as u64,
            0,
            999,
            EventPayload::Test(0),
        ));
        assert!(success, "Push should succeed after popping");
    }
}
