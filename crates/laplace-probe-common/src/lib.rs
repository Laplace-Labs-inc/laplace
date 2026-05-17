#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "std")]
pub mod decoder;

#[cfg(feature = "std")]
pub mod axiom_adapter;

#[cfg(feature = "std")]
pub mod mock;

#[cfg(feature = "std")]
pub mod adapter;

#[cfg(feature = "pipeline")]
pub mod pipeline;

#[cfg(feature = "std")]
pub use decoder::{DecodedProbeEvent, ProbeEventDecoder};

#[cfg(feature = "std")]
pub use axiom_adapter::{
    AxiomEvent, AxiomOp, AxiomResourceId, AxiomStep, AxiomStepBuilder, AxiomThreadId,
    ResourceRegistry, ThreadRegistry, MAX_AXIOM_THREADS,
};
#[cfg(feature = "std")]
pub use adapter::{to_resource_id, to_thread_id};

/// Probe event type discriminant.
///
/// [GHOST CONSTRAINT]: discriminant 1–11 are immutable. `laplace-api ProbeEventDto`,
/// `laplace-probe-sdk`, and `RawProbeEvent decoder` all depend on these values.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeEventType {
    NetRequest = 1,
    NetResponse = 2,
    LockAcquire = 3,
    LockAcquired = 4,
    LockRelease = 5,
    LockContention = 6,
    SchedSwitch = 7,
    ThreadSpawn = 8,
    ThreadExit = 9,
    ConnOpen = 10,
    ConnClose = 11,
    // === Phase 2: RwLock ===
    RwLockReadAcquire = 12,
    RwLockReadRelease = 13,
    RwLockWriteAcquire = 14,
    RwLockWriteRelease = 15,
    // === Phase 2: Atomic ===
    AtomicLoad = 16,
    AtomicStore = 17,
    AtomicRmw = 18,
    // === Phase 2: Semaphore ===
    SemaphoreAcquire = 19,
    SemaphoreRelease = 20,
    // === Phase 2: Channel ===
    ChannelSend = 21,
    ChannelRecv = 22,
}

/// Shared kernel/user-space event structure.
///
/// Must be exactly 128 bytes and `repr(C)`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "std", derive(bytemuck::Pod, bytemuck::Zeroable))]
pub struct RawProbeEvent {
    pub timestamp_ns: u64,   // 8
    pub tid: u32,            // 4
    pub pid: u32,            // 4
    pub event_type: u8,      // 1
    pub l4_proto: u8,        // 1
    pub status_code: u16,    // 2
    pub _pad0: u32,          // 4
    pub resource_id: u64,    // 8
    pub peer_addr: u64,      // 8
    pub peer_port: u32,      // 4
    pub local_port: u32,     // 4
    pub payload_hash: u64,   // 8
    pub payload_len: u32,    // 4
    pub operation_hash: u32, // 4
    pub latency_ns: u64,     // 8
    pub _pad1: u64,          // 8
    pub correlation_id: u64, // 8
    pub cpu_id: u32,         // 4
    pub depth: u32,          // 4
    pub comm: [u8; 16],      // 16
    pub parent_tid: u64,     // 8
    pub _reserved: u64,      // 8
}
// Total: 8+4+4+1+1+2+4+8+8+4+4+8+4+4+8+8+8+4+4+16+8+8 = 128 bytes

const _: () = assert!(
    core::mem::size_of::<RawProbeEvent>() == 128,
    "RawProbeEvent must be exactly 128 bytes"
);
