//! ProbeEventDecoder: translates `RawProbeEvent` (128 bytes, repr C) → `DecodedProbeEvent`.
//!
//! This module is gated behind the `std` feature since `DecodedProbeEvent` uses
//! heap-allocated variants and `Vec` batching. In `no_std` / eBPF contexts, consume
//! `RawProbeEvent` directly from the ring buffer.

#[cfg(feature = "std")]
use crate::{ProbeEventType, RawProbeEvent};

/// Typed, enriched representation of a decoded kernel probe event.
///
/// Decoded from the flat 128-byte `RawProbeEvent`. Each variant retains only
/// the fields relevant to its event type; padding and internal kernel fields
/// are discarded. The DPOR translation layer (`axiom_adapter`) consumes this
/// type to produce `AxiomEvent`s.
#[cfg(feature = "std")]
#[derive(Debug, Clone)]
pub enum DecodedProbeEvent {
    /// Inbound L7 network request — thread is acquiring a connection resource.
    NetworkRequest {
        tid: u32,
        timestamp_ns: u64,
        resource_id: u64,
        operation_hash: u32,
        payload_hash: u64,
        payload_len: u32,
        status_code: u16,
        latency_ns: u64,
        peer_addr: u64,
        peer_port: u32,
    },
    /// Outbound L7 network response — thread is releasing a connection resource.
    NetworkResponse {
        tid: u32,
        timestamp_ns: u64,
        resource_id: u64,
        operation_hash: u32,
        payload_hash: u64,
        payload_len: u32,
        status_code: u16,
        latency_ns: u64,
    },
    /// Thread is requesting a mutex lock (enters contended wait if already held).
    ///
    /// `contention_ns` is derived from `RawProbeEvent::latency_ns`; zero means
    /// the lock was immediately available.
    LockAcquire {
        tid: u32,
        timestamp_ns: u64,
        mutex_addr: u64,
        contention_ns: u64,
    },
    /// Thread successfully acquired a mutex lock after contention.
    ///
    /// `contention_ns` is the total time spent waiting for the lock.
    LockAcquired {
        tid: u32,
        timestamp_ns: u64,
        mutex_addr: u64,
        contention_ns: u64,
    },
    /// Thread released a mutex lock.
    LockRelease {
        tid: u32,
        timestamp_ns: u64,
        mutex_addr: u64,
    },
    /// Thread is blocked — another thread holds the target mutex.
    LockContention {
        tid: u32,
        timestamp_ns: u64,
        mutex_addr: u64,
    },
    /// CPU scheduler context-switched from `prev_tid` to `next_tid`.
    ///
    /// `next_tid` is read from `RawProbeEvent::parent_tid`, which the eBPF
    /// sched_switch hook repurposes to carry the incoming thread's TID.
    SchedSwitch {
        prev_tid: u32,
        next_tid: u32,
        timestamp_ns: u64,
        cpu_id: u32,
    },
    /// A new thread was spawned by `parent_tid`.
    ThreadSpawn {
        parent_tid: u32,
        child_tid: u32,
        timestamp_ns: u64,
    },
    /// A thread exited.
    ThreadExit { tid: u32, timestamp_ns: u64 },
    /// A new network connection was opened (TCP/UDP fd acquired).
    ConnOpen {
        tid: u32,
        timestamp_ns: u64,
        resource_id: u64,
        peer_addr: u64,
        peer_port: u32,
    },
    /// A network connection was closed (fd released).
    ConnClose {
        tid: u32,
        timestamp_ns: u64,
        resource_id: u64,
    },
    /// Thread acquired a shared (read) lock on an RwLock.
    RwLockReadAcquire {
        tid: u32,
        timestamp_ns: u64,
        rwlock_addr: u64,
    },
    /// Thread released a shared (read) lock on an RwLock.
    RwLockReadRelease {
        tid: u32,
        timestamp_ns: u64,
        rwlock_addr: u64,
    },
    /// Thread acquired an exclusive (write) lock on an RwLock.
    RwLockWriteAcquire {
        tid: u32,
        timestamp_ns: u64,
        rwlock_addr: u64,
    },
    /// Thread released an exclusive (write) lock on an RwLock.
    RwLockWriteRelease {
        tid: u32,
        timestamp_ns: u64,
        rwlock_addr: u64,
    },
    /// Thread performed an atomic load.
    AtomicLoad {
        tid: u32,
        timestamp_ns: u64,
        addr: u64,
    },
    /// Thread performed an atomic store.
    AtomicStore {
        tid: u32,
        timestamp_ns: u64,
        addr: u64,
    },
    /// Thread performed an atomic read-modify-write (CAS, fetch_add, etc).
    AtomicRmw {
        tid: u32,
        timestamp_ns: u64,
        addr: u64,
    },
    /// Thread acquired a semaphore.
    SemaphoreAcquire {
        tid: u32,
        timestamp_ns: u64,
        sem_addr: u64,
    },
    /// Thread released a semaphore.
    SemaphoreRelease {
        tid: u32,
        timestamp_ns: u64,
        sem_addr: u64,
    },
    /// Thread sent a message on a channel.
    ChannelSend {
        tid: u32,
        timestamp_ns: u64,
        channel_addr: u64,
    },
    /// Thread received a message from a channel.
    ChannelRecv {
        tid: u32,
        timestamp_ns: u64,
        channel_addr: u64,
    },
}

/// Decodes flat `RawProbeEvent` structs into typed `DecodedProbeEvent` variants.
///
/// The decoder is **stateless**: a single instance can be reused across an
/// unbounded stream of events without resetting.
///
/// # Example
///
/// ```ignore
/// let decoder = ProbeEventDecoder::new();
/// for raw in ring_buffer.drain() {
///     if let Some(decoded) = decoder.decode(&raw) {
///         // forward to AxiomStepBuilder
///     }
/// }
/// ```
#[cfg(feature = "std")]
#[derive(Debug, Default, Clone, Copy)]
pub struct ProbeEventDecoder;

#[cfg(feature = "std")]
impl ProbeEventDecoder {
    /// Creates a new stateless decoder.
    #[inline]
    pub const fn new() -> Self {
        Self
    }

    /// Decodes a single `RawProbeEvent` into a `DecodedProbeEvent`.
    ///
    /// Returns `None` for unrecognised `event_type` values; callers should
    /// skip and continue processing subsequent events.
    pub fn decode(&self, raw: &RawProbeEvent) -> Option<DecodedProbeEvent> {
        let evt = parse_event_type(raw.event_type)?;
        Some(match evt {
            ProbeEventType::NetRequest => DecodedProbeEvent::NetworkRequest {
                tid: raw.tid,
                timestamp_ns: raw.timestamp_ns,
                resource_id: raw.resource_id,
                operation_hash: raw.operation_hash,
                payload_hash: raw.payload_hash,
                payload_len: raw.payload_len,
                status_code: raw.status_code,
                latency_ns: raw.latency_ns,
                peer_addr: raw.peer_addr,
                peer_port: raw.peer_port,
            },
            ProbeEventType::NetResponse => DecodedProbeEvent::NetworkResponse {
                tid: raw.tid,
                timestamp_ns: raw.timestamp_ns,
                resource_id: raw.resource_id,
                operation_hash: raw.operation_hash,
                payload_hash: raw.payload_hash,
                payload_len: raw.payload_len,
                status_code: raw.status_code,
                latency_ns: raw.latency_ns,
            },
            ProbeEventType::LockAcquire => DecodedProbeEvent::LockAcquire {
                tid: raw.tid,
                timestamp_ns: raw.timestamp_ns,
                // resource_id doubles as mutex address in lock events
                mutex_addr: raw.resource_id,
                contention_ns: raw.latency_ns,
            },
            ProbeEventType::LockAcquired => DecodedProbeEvent::LockAcquired {
                tid: raw.tid,
                timestamp_ns: raw.timestamp_ns,
                mutex_addr: raw.resource_id,
                contention_ns: raw.latency_ns,
            },
            ProbeEventType::LockRelease => DecodedProbeEvent::LockRelease {
                tid: raw.tid,
                timestamp_ns: raw.timestamp_ns,
                mutex_addr: raw.resource_id,
            },
            ProbeEventType::LockContention => DecodedProbeEvent::LockContention {
                tid: raw.tid,
                timestamp_ns: raw.timestamp_ns,
                mutex_addr: raw.resource_id,
            },
            ProbeEventType::SchedSwitch => DecodedProbeEvent::SchedSwitch {
                prev_tid: raw.tid,
                // eBPF sched_switch repurposes parent_tid to carry the incoming thread's TID
                next_tid: raw.parent_tid as u32,
                timestamp_ns: raw.timestamp_ns,
                cpu_id: raw.cpu_id,
            },
            ProbeEventType::ThreadSpawn => DecodedProbeEvent::ThreadSpawn {
                parent_tid: raw.parent_tid as u32,
                child_tid: raw.tid,
                timestamp_ns: raw.timestamp_ns,
            },
            ProbeEventType::ThreadExit => DecodedProbeEvent::ThreadExit {
                tid: raw.tid,
                timestamp_ns: raw.timestamp_ns,
            },
            ProbeEventType::ConnOpen => DecodedProbeEvent::ConnOpen {
                tid: raw.tid,
                timestamp_ns: raw.timestamp_ns,
                resource_id: raw.resource_id,
                peer_addr: raw.peer_addr,
                peer_port: raw.peer_port,
            },
            ProbeEventType::ConnClose => DecodedProbeEvent::ConnClose {
                tid: raw.tid,
                timestamp_ns: raw.timestamp_ns,
                resource_id: raw.resource_id,
            },
            ProbeEventType::RwLockReadAcquire => DecodedProbeEvent::RwLockReadAcquire {
                tid: raw.tid,
                timestamp_ns: raw.timestamp_ns,
                rwlock_addr: raw.resource_id,
            },
            ProbeEventType::RwLockReadRelease => DecodedProbeEvent::RwLockReadRelease {
                tid: raw.tid,
                timestamp_ns: raw.timestamp_ns,
                rwlock_addr: raw.resource_id,
            },
            ProbeEventType::RwLockWriteAcquire => DecodedProbeEvent::RwLockWriteAcquire {
                tid: raw.tid,
                timestamp_ns: raw.timestamp_ns,
                rwlock_addr: raw.resource_id,
            },
            ProbeEventType::RwLockWriteRelease => DecodedProbeEvent::RwLockWriteRelease {
                tid: raw.tid,
                timestamp_ns: raw.timestamp_ns,
                rwlock_addr: raw.resource_id,
            },
            ProbeEventType::AtomicLoad => DecodedProbeEvent::AtomicLoad {
                tid: raw.tid,
                timestamp_ns: raw.timestamp_ns,
                addr: raw.resource_id,
            },
            ProbeEventType::AtomicStore => DecodedProbeEvent::AtomicStore {
                tid: raw.tid,
                timestamp_ns: raw.timestamp_ns,
                addr: raw.resource_id,
            },
            ProbeEventType::AtomicRmw => DecodedProbeEvent::AtomicRmw {
                tid: raw.tid,
                timestamp_ns: raw.timestamp_ns,
                addr: raw.resource_id,
            },
            ProbeEventType::SemaphoreAcquire => DecodedProbeEvent::SemaphoreAcquire {
                tid: raw.tid,
                timestamp_ns: raw.timestamp_ns,
                sem_addr: raw.resource_id,
            },
            ProbeEventType::SemaphoreRelease => DecodedProbeEvent::SemaphoreRelease {
                tid: raw.tid,
                timestamp_ns: raw.timestamp_ns,
                sem_addr: raw.resource_id,
            },
            ProbeEventType::ChannelSend => DecodedProbeEvent::ChannelSend {
                tid: raw.tid,
                timestamp_ns: raw.timestamp_ns,
                channel_addr: raw.resource_id,
            },
            ProbeEventType::ChannelRecv => DecodedProbeEvent::ChannelRecv {
                tid: raw.tid,
                timestamp_ns: raw.timestamp_ns,
                channel_addr: raw.resource_id,
            },
        })
    }

    /// Decodes a slice of raw events in order, silently skipping unknown types.
    pub fn decode_batch(&self, events: &[RawProbeEvent]) -> Vec<DecodedProbeEvent> {
        events.iter().filter_map(|e| self.decode(e)).collect()
    }
}

/// Converts a raw `u8` discriminant to a `ProbeEventType`.
/// Kept as a free function so it can be inlined away in hot decode loops.
#[cfg(feature = "std")]
#[inline]
fn parse_event_type(raw: u8) -> Option<ProbeEventType> {
    match raw {
        1 => Some(ProbeEventType::NetRequest),
        2 => Some(ProbeEventType::NetResponse),
        3 => Some(ProbeEventType::LockAcquire),
        4 => Some(ProbeEventType::LockAcquired),
        5 => Some(ProbeEventType::LockRelease),
        6 => Some(ProbeEventType::LockContention),
        7 => Some(ProbeEventType::SchedSwitch),
        8 => Some(ProbeEventType::ThreadSpawn),
        9 => Some(ProbeEventType::ThreadExit),
        10 => Some(ProbeEventType::ConnOpen),
        11 => Some(ProbeEventType::ConnClose),
        12 => Some(ProbeEventType::RwLockReadAcquire),
        13 => Some(ProbeEventType::RwLockReadRelease),
        14 => Some(ProbeEventType::RwLockWriteAcquire),
        15 => Some(ProbeEventType::RwLockWriteRelease),
        16 => Some(ProbeEventType::AtomicLoad),
        17 => Some(ProbeEventType::AtomicStore),
        18 => Some(ProbeEventType::AtomicRmw),
        19 => Some(ProbeEventType::SemaphoreAcquire),
        20 => Some(ProbeEventType::SemaphoreRelease),
        21 => Some(ProbeEventType::ChannelSend),
        22 => Some(ProbeEventType::ChannelRecv),
        _ => None,
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::RawProbeEvent;

    fn zeroed_raw() -> RawProbeEvent {
        // SAFETY: RawProbeEvent is repr(C) with no padding-sensitive invariants;
        // zeroing produces a valid (though semantically empty) instance.
        unsafe { core::mem::zeroed() }
    }

    #[test]
    fn decode_lock_acquire_maps_resource_id_to_mutex_addr() {
        let decoder = ProbeEventDecoder::new();
        let mut raw = zeroed_raw();
        raw.event_type = 3; // LockAcquire
        raw.tid = 42;
        raw.resource_id = 0xDEAD_BEEF_0000_0000;
        raw.latency_ns = 1_000;
        raw.timestamp_ns = 500;

        match decoder.decode(&raw).unwrap() {
            DecodedProbeEvent::LockAcquire {
                tid,
                mutex_addr,
                contention_ns,
                timestamp_ns,
            } => {
                assert_eq!(tid, 42);
                assert_eq!(mutex_addr, 0xDEAD_BEEF_0000_0000);
                assert_eq!(contention_ns, 1_000);
                assert_eq!(timestamp_ns, 500);
            }
            other => panic!("unexpected variant: {:?}", other),
        }
    }

    #[test]
    fn decode_sched_switch_uses_parent_tid_as_next_tid() {
        let decoder = ProbeEventDecoder::new();
        let mut raw = zeroed_raw();
        raw.event_type = 7; // SchedSwitch
        raw.tid = 10; // prev_tid
        raw.parent_tid = 20; // next_tid (repurposed field)
        raw.cpu_id = 3;

        match decoder.decode(&raw).unwrap() {
            DecodedProbeEvent::SchedSwitch {
                prev_tid,
                next_tid,
                cpu_id,
                ..
            } => {
                assert_eq!(prev_tid, 10);
                assert_eq!(next_tid, 20);
                assert_eq!(cpu_id, 3);
            }
            other => panic!("unexpected variant: {:?}", other),
        }
    }

    #[test]
    fn decode_returns_none_for_unknown_event_type() {
        let decoder = ProbeEventDecoder::new();
        let mut raw = zeroed_raw();
        raw.event_type = 255;
        assert!(decoder.decode(&raw).is_none());
    }

    #[test]
    fn decode_batch_skips_unknown_types() {
        let decoder = ProbeEventDecoder::new();
        let mut events = [zeroed_raw(); 3];
        events[0].event_type = 5; // LockRelease — valid
        events[1].event_type = 99; // unknown — skipped
        events[2].event_type = 9; // ThreadExit — valid

        let decoded = decoder.decode_batch(&events);
        assert_eq!(decoded.len(), 2);
        assert!(matches!(decoded[0], DecodedProbeEvent::LockRelease { .. }));
        assert!(matches!(decoded[1], DecodedProbeEvent::ThreadExit { .. }));
    }
}
