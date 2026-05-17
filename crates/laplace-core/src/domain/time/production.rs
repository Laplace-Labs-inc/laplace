//! Production-grade clock backend implementation.
//!
//! This module provides the `ProductionBackend`, optimized for high-performance
//! discrete event simulation. It uses a binary heap for efficient event queue
//! management and is suitable for all production scenarios requiring fast event
//! scheduling and processing.

use super::types::{LamportClock, ScheduledEvent, VirtualTimeNs};
use super::ClockBackend;
use std::collections::BinaryHeap;

/// High-performance clock backend using a binary heap for event queue.
///
/// The `ProductionBackend` is the default implementation of `ClockBackend`,
/// optimized for speed and scalability. It maintains:
///
/// - Virtual time in nanoseconds for precise temporal ordering
/// - A Lamport logical clock for causality tracking across events
/// - A priority queue (binary heap) for efficient event management
///
/// # Performance Characteristics
///
/// - **Schedule Event**: O(log n) where n is the queue size
/// - **Process Event**: O(log n) for heap extraction
/// - **Current Time/Lamport**: O(1)
/// - **Memory**: Linear in queue size (typical discrete event simulation)
///
/// # Design Notes
///
/// The binary heap is a standard choice for discrete event simulation because
/// it provides O(log n) performance for both insertion and extraction of the
/// minimum-priority element. Since `ScheduledEvent` implements ordering with
/// earlier times having higher priority, the heap maintains events in the
/// correct order without additional sorting overhead.
///
/// # Thread Safety
///
/// This backend is not thread-safe by itself. For concurrent access, wrap it
/// in a `Mutex` or `Arc<Mutex<_>>` as needed.
#[derive(Debug)]
pub struct ProductionBackend {
    /// Current virtual time in nanoseconds.
    ///
    /// Advances only when events are processed (event-driven simulation).
    /// Starts at zero and is monotonically non-decreasing.
    current_time: VirtualTimeNs,

    /// Current Lamport logical clock value.
    ///
    /// Incremented on each event to provide a total ordering of events.
    /// When the event queue empties, this is reset to zero.
    lamport: LamportClock,

    /// Priority queue of scheduled events.
    ///
    /// Uses a binary heap to maintain events in order of priority:
    /// 1. Earliest scheduled time (minimum time has highest priority)
    /// 2. Lowest Lamport clock value (for simultaneous events)
    /// 3. Smallest event ID (for deterministic tie-breaking)
    queue: BinaryHeap<ScheduledEvent>,
}

impl ProductionBackend {
    /// Create a new production clock backend.
    ///
    /// Initializes the backend with zero time, zero Lamport clock, and an
    /// empty event queue. The backend is ready for event scheduling immediately.
    ///
    /// # Returns
    ///
    /// A new `ProductionBackend` initialized to its starting state.
    pub fn new() -> Self {
        Self {
            current_time: 0,
            lamport: 0,
            queue: BinaryHeap::new(),
        }
    }

    /// Get the current capacity of the event queue.
    ///
    /// # Returns
    ///
    /// The allocated capacity of the internal BinaryHeap.
    pub fn queue_capacity(&self) -> usize {
        self.queue.capacity()
    }
}

impl Default for ProductionBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl ClockBackend for ProductionBackend {
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
        // BinaryHeap never fails to add an event (it grows dynamically).
        // Always returns true to satisfy the trait contract.
        self.queue.push(event);
        true
    }

    fn pop_event(&mut self) -> Option<ScheduledEvent> {
        let event = self.queue.pop();

        // Reset Lamport clock when queue becomes empty
        if self.queue.is_empty() && event.is_some() {
            self.reset_lamport();
        }

        event
    }

    fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    fn queue_len(&self) -> usize {
        self.queue.len()
    }

    fn reset(&mut self) {
        self.current_time = 0;
        self.lamport = 0;
        self.queue.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::memory::CoreId;
    use crate::domain::time::EventPayload;

    #[test]
    fn test_production_backend_creation() {
        let backend = ProductionBackend::new();
        assert_eq!(backend.current_time(), 0);
        assert_eq!(backend.current_lamport(), 0);
        assert!(backend.is_empty());
    }

    #[test]
    fn test_production_backend_push_event() {
        let mut backend = ProductionBackend::new();
        let core0 = CoreId::new(0);

        let event = ScheduledEvent::new(100, 0, 1, EventPayload::MemoryFence { core: core0 });

        assert!(backend.push_event(event));
        assert_eq!(backend.queue_len(), 1);
        assert!(!backend.is_empty());
    }

    #[test]
    fn test_production_backend_pop_event() {
        let mut backend = ProductionBackend::new();
        let core0 = CoreId::new(0);

        let event = ScheduledEvent::new(100, 0, 1, EventPayload::MemoryFence { core: core0 });

        backend.push_event(event.clone());
        assert_eq!(backend.queue_len(), 1);

        let popped = backend.pop_event();
        assert!(popped.is_some());
        assert_eq!(popped.unwrap().event_id, 1);
        assert!(backend.is_empty());
    }

    #[test]
    fn test_production_backend_event_ordering() {
        let mut backend = ProductionBackend::new();

        // Add events in reverse order
        let e1 = ScheduledEvent::new(300, 0, 3, EventPayload::Test(0));
        let e2 = ScheduledEvent::new(100, 0, 1, EventPayload::Test(0));
        let e3 = ScheduledEvent::new(200, 0, 2, EventPayload::Test(0));

        backend.push_event(e1);
        backend.push_event(e2);
        backend.push_event(e3);

        // Should pop in order of scheduled time
        let first = backend.pop_event().unwrap();
        assert_eq!(
            first.scheduled_at_ns, 100,
            "First event should be at time 100"
        );

        let second = backend.pop_event().unwrap();
        assert_eq!(
            second.scheduled_at_ns, 200,
            "Second event should be at time 200"
        );

        let third = backend.pop_event().unwrap();
        assert_eq!(
            third.scheduled_at_ns, 300,
            "Third event should be at time 300"
        );
    }

    #[test]
    fn test_production_backend_lamport_ordering() {
        let mut backend = ProductionBackend::new();

        // Events at same time with different Lamport values
        let e1 = ScheduledEvent::new(100, 2, 1, EventPayload::Test(0));
        let e2 = ScheduledEvent::new(100, 1, 2, EventPayload::Test(0));

        backend.push_event(e1);
        backend.push_event(e2);

        // Should pop lower Lamport first
        let first = backend.pop_event().unwrap();
        assert_eq!(first.lamport, 1, "Event with lower Lamport should be first");

        let second = backend.pop_event().unwrap();
        assert_eq!(
            second.lamport, 2,
            "Event with higher Lamport should be second"
        );
    }

    #[test]
    fn test_production_backend_time_advance() {
        let mut backend = ProductionBackend::new();

        assert_eq!(backend.current_time(), 0);
        backend.set_time(1000);
        assert_eq!(backend.current_time(), 1000);
    }

    #[test]
    fn test_production_backend_lamport_increment() {
        let mut backend = ProductionBackend::new();

        assert_eq!(backend.current_lamport(), 0);
        let new_lamport = backend.increment_lamport();
        assert_eq!(new_lamport, 1);
        assert_eq!(backend.current_lamport(), 1);
    }

    #[test]
    fn test_production_backend_lamport_reset() {
        let mut backend = ProductionBackend::new();

        backend.increment_lamport();
        assert_eq!(backend.current_lamport(), 1);

        backend.reset_lamport();
        assert_eq!(backend.current_lamport(), 0);
    }

    #[test]
    fn test_production_backend_reset() {
        let mut backend = ProductionBackend::new();
        let core0 = CoreId::new(0);

        backend.set_time(5000);
        backend.increment_lamport();
        let event = ScheduledEvent::new(1000, 1, 1, EventPayload::MemoryFence { core: core0 });
        backend.push_event(event);

        assert_eq!(backend.current_time(), 5000);
        assert_eq!(backend.current_lamport(), 1);
        assert_eq!(backend.queue_len(), 1);

        backend.reset();

        assert_eq!(backend.current_time(), 0);
        assert_eq!(backend.current_lamport(), 0);
        assert_eq!(backend.queue_len(), 0);
    }

    #[test]
    fn test_production_backend_lamport_reset_on_empty() {
        let mut backend = ProductionBackend::new();

        let event1 = ScheduledEvent::new(100, 0, 1, EventPayload::Test(0));
        let event2 = ScheduledEvent::new(200, 0, 2, EventPayload::Test(0));

        backend.push_event(event1);
        backend.push_event(event2);
        backend.increment_lamport(); // Simulate processing an event

        // Pop both events
        backend.pop_event();
        assert!(!backend.is_empty(), "Queue should still have one event");
        assert_eq!(
            backend.current_lamport(),
            1,
            "Lamport should not reset yet (queue not empty)"
        );

        backend.pop_event();
        assert!(backend.is_empty(), "Queue should now be empty");
        assert_eq!(
            backend.current_lamport(),
            0,
            "Lamport should reset when queue becomes empty"
        );
    }
}
