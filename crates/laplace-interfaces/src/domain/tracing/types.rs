//! Core type definitions for deterministic event tracing.
//!
//! Foundational types for capturing and analysing causal relationships in the
//! Laplace simulation engine. All types target stack allocation, zero-cost
//! abstraction, and deterministic replay across runs.
//!
//! **Principle**: Deterministic Context — all operations track causality through
//! Lamport timestamps; no implicit state propagation.

use crate::domain::memory::Address;
use std::fmt;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Maximum number of concurrent threads the tracer can observe.
///
/// Compile-time constant enabling array-based storage with no heap allocation.
/// Increase if simulating more than 16 threads simultaneously.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Tracing",
        link = "LEP-0014-laplace-interfaces-deterministic_tracing"
    )
)]
pub const MAX_THREADS: usize = 16;

/// Thread identifier in the tracing domain.
///
/// Must be in `[0, MAX_THREADS)` so it can serve as a direct array index into
/// per-thread bookkeeping structures, giving zero-cost lookup.
///
/// # Note
/// This type is `u32`-based and distinct from `scheduler::ThreadId` (`usize`)
/// and `resource::ThreadId` (`usize`).
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Tracing",
        link = "LEP-0014-laplace-interfaces-deterministic_tracing"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct ThreadId(pub u32);

impl ThreadId {
    /// Create a new `ThreadId`, validating bounds in debug builds.
    ///
    /// - `id`: Zero-based thread index; must be `< MAX_THREADS` in debug mode.
    ///
    /// Returns `ThreadId(id)`.
    ///
    /// # Panics
    /// Panics in debug builds if `id >= MAX_THREADS`.
    #[inline(always)]
    pub fn new(id: u32) -> Self {
        debug_assert!(
            (id as usize) < MAX_THREADS,
            "ThreadId {} out of bounds (max: {})",
            id,
            MAX_THREADS
        );
        ThreadId(id)
    }

    /// Convert to a `usize` array index (zero-cost).
    #[inline(always)]
    pub fn as_index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for ThreadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "T{}", self.0)
    }
}

/// Lamport logical timestamp for causality ordering.
///
/// Implements standard Lamport clock semantics:
/// - Local events: increment by 1.
/// - Message reception: `new = max(local, remote) + 1`.
///
/// Enables partial-order analysis of concurrent simulation events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct LamportTimestamp(pub u64);

impl LamportTimestamp {
    /// The initial timestamp value (zero).
    pub const ZERO: Self = LamportTimestamp(0);

    /// Increment the timestamp by one (local event).
    ///
    /// Uses wrapping arithmetic to avoid panics on overflow.
    #[inline(always)]
    pub fn increment(&mut self) {
        self.0 = self.0.wrapping_add(1);
    }

    /// Synchronize with `remote` (message arrival).
    ///
    /// - `remote`: Timestamp received from another thread.
    ///
    /// Sets `self` to `max(self, remote) + 1` (saturating).
    #[inline(always)]
    pub fn sync(&mut self, remote: LamportTimestamp) {
        self.0 = core::cmp::max(self.0, remote.0).saturating_add(1);
    }
}

impl fmt::Display for LamportTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{}", self.0)
    }
}

/// Metadata attached to every simulation event.
///
/// Carries the minimum information needed for causality analysis: global
/// ordering timestamp, generating thread, and per-thread sequence number.
///
/// 24 bytes, `#[repr(C)]` for cache-line efficiency.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Tracing",
        link = "LEP-0014-laplace-interfaces-deterministic_tracing"
    )
)]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct EventMetadata {
    /// Lamport timestamp when this event occurred.
    pub timestamp: LamportTimestamp,
    /// Thread that generated this event.
    pub thread_id: ThreadId,
    /// Compiler-inserted padding (4 bytes, not user-visible).
    _pad: u32,
    /// Sequence number within the thread for total ordering of local events.
    pub seq_num: u64,
}

impl EventMetadata {
    /// Construct new event metadata.
    ///
    /// - `timestamp`: Lamport timestamp for this event.
    /// - `thread_id`: Thread that generated the event.
    /// - `seq_num`: Per-thread sequence number.
    ///
    /// Returns a fully initialised `EventMetadata`.
    #[inline(always)]
    pub fn new(timestamp: LamportTimestamp, thread_id: ThreadId, seq_num: u64) -> Self {
        Self {
            timestamp,
            thread_id,
            _pad: 0,
            seq_num,
        }
    }
}

/// Memory fence ordering semantics.
///
/// Corresponds to the standard acquire/release/sequential-consistency fence
/// taxonomy used in concurrent programming memory models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FenceType {
    /// Acquire — prevents reordering of subsequent reads/writes above this fence.
    Acquire = 0,
    /// Release — prevents reordering of prior reads/writes below this fence.
    Release = 1,
    /// Sequential consistency — full bidirectional memory barrier.
    SeqCst = 2,
}

impl fmt::Display for FenceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FenceType::Acquire => write!(f, "Acquire"),
            FenceType::Release => write!(f, "Release"),
            FenceType::SeqCst => write!(f, "SeqCst"),
        }
    }
}

/// A single traced memory operation.
///
/// Distinguishes reads, writes, buffer flushes, and fences to enable detailed
/// memory model analysis (TSO, PSO, etc.).
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Tracing",
        link = "LEP-0014-laplace-interfaces-deterministic_tracing"
    )
)]
#[derive(Debug, Clone, Copy)]
pub enum MemoryOperation {
    /// Load from memory.
    Read {
        /// Address being read.
        addr: Address,
        /// Value observed (for causality chains).
        value: u64,
        /// `true` if the value came from the store buffer rather than main memory.
        cache_hit: bool,
    },

    /// Store to memory.
    Write {
        /// Address being written.
        addr: Address,
        /// Value being stored.
        value: u64,
        /// `true` if the write entered the store buffer (not immediately visible to other cores).
        buffered: bool,
    },

    /// Store buffer entry drained to main memory.
    BufferFlush {
        /// Address being flushed.
        addr: Address,
        /// Value written to main memory.
        value: u64,
    },

    /// Memory barrier operation.
    Fence {
        /// The type of ordering enforced by this fence.
        fence_type: FenceType,
    },
}

impl fmt::Display for MemoryOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryOperation::Read {
                addr,
                value,
                cache_hit,
            } => write!(
                f,
                "Read(0x{:x}) = {} {}",
                addr.0,
                value,
                if *cache_hit { "[cache]" } else { "[main]" }
            ),
            MemoryOperation::Write {
                addr,
                value,
                buffered,
            } => write!(
                f,
                "Write(0x{:x}, {}) {}",
                addr.0,
                value,
                if *buffered { "[buffered]" } else { "[direct]" }
            ),
            MemoryOperation::BufferFlush { addr, value } => {
                write!(f, "Flush(0x{:x}, {})", addr.0, value)
            }
            MemoryOperation::Fence { fence_type } => write!(f, "Fence({})", fence_type),
        }
    }
}

/// A traced synchronization primitive event.
///
/// Records interactions with locks, condition variables, and other primitives
/// that establish causality edges between threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyncEvent {
    /// Mutex lock acquired.
    MutexLock {
        /// Identifier of the lock object.
        lock_id: u64,
    },
    /// Mutex lock released.
    MutexUnlock {
        /// Identifier of the lock object.
        lock_id: u64,
    },
    /// Condition variable wait initiated.
    CondVarWait {
        /// Identifier of the condition variable.
        cv_id: u64,
    },
    /// Condition variable signal sent.
    CondVarSignal {
        /// Identifier of the condition variable.
        cv_id: u64,
    },
}

impl fmt::Display for SyncEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncEvent::MutexLock { lock_id } => write!(f, "MutexLock({})", lock_id),
            SyncEvent::MutexUnlock { lock_id } => write!(f, "MutexUnlock({})", lock_id),
            SyncEvent::CondVarWait { cv_id } => write!(f, "CondVarWait({})", cv_id),
            SyncEvent::CondVarSignal { cv_id } => write!(f, "CondVarSignal({})", cv_id),
        }
    }
}

/// An explicit update to a thread's logical clock.
///
/// Used in verification and testing to assert monotonic clock progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClockEvent {
    /// Timestamp before the update.
    pub prev_timestamp: LamportTimestamp,
    /// Timestamp after the update.
    pub new_timestamp: LamportTimestamp,
}

impl fmt::Display for ClockEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ClockTick({} -> {})",
            self.prev_timestamp, self.new_timestamp
        )
    }
}

/// A complete simulation event — the canonical unit of the trace log.
///
/// Each variant is self-contained: no external lookups are needed to interpret
/// the event, supporting zero-copy serialisation and formal verification.
///
/// Target: 64 bytes (`#[repr(C)]`, single cache line).
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Tracing",
        link = "LEP-0014-laplace-interfaces-deterministic_tracing"
    )
)]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub enum SimulationEvent {
    /// Explicit Lamport clock advancement (used primarily in verification).
    ClockTick {
        /// Common event metadata.
        meta: EventMetadata,
        /// Clock update details.
        event: ClockEvent,
    },

    /// Memory operation (read, write, fence, flush).
    Memory {
        /// Common event metadata.
        meta: EventMetadata,
        /// The specific memory operation.
        operation: MemoryOperation,
    },

    /// Synchronization primitive event (lock, unlock, condition variable).
    Synchronization {
        /// Common event metadata.
        meta: EventMetadata,
        /// The synchronization operation.
        sync_event: SyncEvent,
    },

    /// Thread spawned — establishes parent→child causality edge.
    ThreadSpawn {
        /// Common event metadata.
        meta: EventMetadata,
        /// ID of the newly created child thread.
        child_id: ThreadId,
    },

    /// Thread joined — establishes child→parent causality edge.
    ThreadJoin {
        /// Common event metadata.
        meta: EventMetadata,
        /// ID of the thread that was joined (waited for).
        child_id: ThreadId,
    },
}

impl SimulationEvent {
    /// Return a reference to the common metadata of any event variant.
    #[inline(always)]
    pub fn metadata(&self) -> &EventMetadata {
        match self {
            SimulationEvent::ClockTick { meta, .. }
            | SimulationEvent::Memory { meta, .. }
            | SimulationEvent::Synchronization { meta, .. }
            | SimulationEvent::ThreadSpawn { meta, .. }
            | SimulationEvent::ThreadJoin { meta, .. } => meta,
        }
    }

    /// Return the Lamport timestamp of this event.
    #[inline(always)]
    pub fn timestamp(&self) -> LamportTimestamp {
        self.metadata().timestamp
    }

    /// Return the [`ThreadId`] that generated this event.
    #[inline(always)]
    pub fn thread_id(&self) -> ThreadId {
        self.metadata().thread_id
    }

    /// Return `true` if this event happened before `other` in Lamport order.
    ///
    /// - `other`: The event to compare against.
    #[inline(always)]
    pub fn happens_before(&self, other: &SimulationEvent) -> bool {
        self.timestamp() < other.timestamp()
    }
}

impl fmt::Display for SimulationEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let meta = self.metadata();
        write!(f, "[{}@{}] ", meta.thread_id, meta.timestamp)?;
        match self {
            SimulationEvent::ClockTick { event, .. } => write!(f, "{}", event),
            SimulationEvent::Memory { operation, .. } => write!(f, "{}", operation),
            SimulationEvent::Synchronization { sync_event, .. } => write!(f, "{}", sync_event),
            SimulationEvent::ThreadSpawn { child_id, .. } => write!(f, "Spawn -> {}", child_id),
            SimulationEvent::ThreadJoin { child_id, .. } => write!(f, "Join <- {}", child_id),
        }
    }
}
