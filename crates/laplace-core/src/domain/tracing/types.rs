//! Tracing type definitions — re-exported from `laplace-interfaces`
//!
//! All canonical types live in `laplace_interfaces::domain::tracing::types`.
//! This file re-exports them so that code within `laplace-core` can continue to use
//! `crate::domain::tracing::{LamportTimestamp, SimulationEvent, …}`.

pub use laplace_interfaces::domain::tracing::types::{
    ClockEvent, EventMetadata, FenceType, LamportTimestamp, MemoryOperation, SimulationEvent,
    SyncEvent, ThreadId, MAX_THREADS,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lamport_increment() {
        let mut ts = LamportTimestamp(5);
        ts.increment();
        assert_eq!(ts.0, 6);
    }

    #[test]
    fn test_lamport_sync() {
        let mut ts1 = LamportTimestamp(5);
        let ts2 = LamportTimestamp(10);
        ts1.sync(ts2);
        assert_eq!(ts1.0, 11);
    }

    #[test]
    fn test_lamport_sync_self() {
        let mut ts = LamportTimestamp(10);
        ts.sync(LamportTimestamp(5));
        assert_eq!(ts.0, 11);
    }

    #[test]
    fn test_thread_id_bounds() {
        let tid = ThreadId::new(5);
        assert_eq!(tid.as_index(), 5);
    }

    #[test]
    fn test_event_metadata_layout() {
        let meta = EventMetadata::new(LamportTimestamp(42), ThreadId::new(1), 7);
        assert_eq!(meta.timestamp.0, 42);
        assert_eq!(meta.thread_id.0, 1);
        assert_eq!(meta.seq_num, 7);
    }

    #[test]
    fn test_happens_before() {
        let meta1 = EventMetadata::new(LamportTimestamp(5), ThreadId::new(0), 0);
        let meta2 = EventMetadata::new(LamportTimestamp(10), ThreadId::new(0), 1);

        let e1 = SimulationEvent::Memory {
            meta: meta1,
            operation: MemoryOperation::Fence {
                fence_type: FenceType::SeqCst,
            },
        };

        let e2 = SimulationEvent::Memory {
            meta: meta2,
            operation: MemoryOperation::Fence {
                fence_type: FenceType::SeqCst,
            },
        };

        assert!(e1.happens_before(&e2));
        assert!(!e2.happens_before(&e1));
    }
}
