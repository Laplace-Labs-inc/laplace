// SPDX-License-Identifier: Apache-2.0
//! Time system type definitions — re-exported from `laplace-interfaces`
//!
//! All canonical types live in `laplace_interfaces::domain::time::types`.
//! This file re-exports them so that code within `laplace-core` can continue to use
//! `crate::domain::time::{VirtualTimeNs, LamportClock, …}`.

pub use laplace_interfaces::domain::time::types::{
    EventId, EventPayload, LamportClock, ScheduledEvent, TimeMode, VirtualTimeNs,
};

#[cfg(test)]
mod tests {
    use super::*;
    use laplace_interfaces::domain::memory::CoreId;

    #[test]
    fn test_event_ordering_by_time() {
        let core0 = CoreId::new(0);
        let e1 = ScheduledEvent::new(100, 1, 1, EventPayload::MemoryFence { core: core0 });
        let e2 = ScheduledEvent::new(200, 1, 2, EventPayload::MemoryFence { core: core0 });

        assert!(e1 > e2, "Earlier events should have higher priority");
    }

    #[test]
    fn test_event_ordering_by_lamport() {
        let core0 = CoreId::new(0);
        let e1 = ScheduledEvent::new(100, 1, 1, EventPayload::MemoryFence { core: core0 });
        let e2 = ScheduledEvent::new(100, 2, 2, EventPayload::MemoryFence { core: core0 });

        assert!(
            e1 > e2,
            "Events at same time should be ordered by Lamport clock"
        );
    }

    #[test]
    fn test_event_ordering_deterministic() {
        let core0 = CoreId::new(0);
        let e1 = ScheduledEvent::new(100, 1, 1, EventPayload::MemoryFence { core: core0 });
        let e2 = ScheduledEvent::new(100, 1, 2, EventPayload::MemoryFence { core: core0 });

        assert!(
            e1 > e2,
            "Events at same time/Lamport should be ordered by event_id"
        );
    }

    #[test]
    fn test_event_equality_by_id() {
        let core0 = CoreId::new(0);
        let e1 = ScheduledEvent::new(100, 1, 1, EventPayload::MemoryFence { core: core0 });
        let e2 = ScheduledEvent::new(200, 2, 1, EventPayload::MemoryFence { core: core0 });

        assert_eq!(e1, e2, "Events with same ID should be equal");
    }
}
