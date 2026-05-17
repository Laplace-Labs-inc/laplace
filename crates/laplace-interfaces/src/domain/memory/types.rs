//! Memory Model Type Definitions
//!
//! Canonical types for the Laplace memory abstraction layer. These types correspond
//! directly to the TLA+ specification in `SimulatedMemory.tla` and are used by both
//! `laplace-core` (implementations) and `laplace-kraken` (simulation engine).

use std::fmt;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Memory address.
///
/// Used by [`super::traits::MemoryBackend`] to index main memory and store buffers.
///
/// # TLA+ Correspondence
/// Element of the `Addresses` set in `SimulatedMemory.tla`.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Memory",
        link = "LEP-0005-laplace-interfaces-memory_model"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Address(pub usize);

impl Address {
    /// Create a new `Address` from a raw `usize`.
    ///
    /// - `addr`: Raw address index.
    ///
    /// Returns `Address(addr)`.
    pub const fn new(addr: usize) -> Self {
        Self(addr)
    }

    /// Return the raw `usize` value of this address.
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

impl From<usize> for Address {
    fn from(v: usize) -> Self {
        Self(v)
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{:x}", self.0)
    }
}

/// Memory value — an arbitrary 64-bit word.
///
/// Used as the unit of data stored at each [`Address`] in main memory and store buffers.
///
/// # TLA+ Correspondence
/// Element of the `Values` set in `SimulatedMemory.tla`.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Memory",
        link = "LEP-0005-laplace-interfaces-memory_model"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct Value(pub u64);

impl Value {
    /// Create a new `Value` from a raw `u64`.
    ///
    /// - `val`: Raw 64-bit integer.
    ///
    /// Returns `Value(val)`.
    pub const fn new(val: u64) -> Self {
        Self(val)
    }

    /// Return the raw `u64` representation of this value.
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl From<u64> for Value {
    fn from(v: u64) -> Self {
        Self(v)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// CPU core identifier.
///
/// Each core owns an independent store buffer and executes memory operations
/// in parallel with other cores. Used to index store buffers in
/// [`super::traits::MemoryBackend`].
///
/// # TLA+ Correspondence
/// Element of the `Cores` set in `SimulatedMemory.tla`.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Memory",
        link = "LEP-0005-laplace-interfaces-memory_model"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct CoreId(pub usize);

impl CoreId {
    /// Create a new `CoreId` from a raw index.
    ///
    /// - `id`: Zero-based core index.
    ///
    /// Returns `CoreId(id)`.
    pub const fn new(id: usize) -> Self {
        Self(id)
    }

    /// Return the raw `usize` index of this core.
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

impl From<usize> for CoreId {
    fn from(v: usize) -> Self {
        Self(v)
    }
}

impl fmt::Display for CoreId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Core({})", self.0)
    }
}

/// A pending write entry in a core's store buffer.
///
/// Represents one queued write that has not yet been committed to main memory.
/// Entries are drained FIFO via [`super::traits::MemoryBackend::buffer_pop`].
///
/// # TLA+ Correspondence
/// ```tla
/// [addr: Address, val: Value]
/// ```
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Memory",
        link = "LEP-0005-laplace-interfaces-memory_model"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreEntry {
    /// Target memory address for the pending write.
    pub addr: Address,

    /// Value to be written to [`addr`](StoreEntry::addr) when the entry is drained.
    pub val: Value,
}

impl StoreEntry {
    /// Create a new store buffer entry.
    ///
    /// - `addr`: Memory address to write to.
    /// - `val`: Value to write.
    ///
    /// Returns a `StoreEntry` representing the pending write.
    pub fn new(addr: Address, val: Value) -> Self {
        Self { addr, val }
    }
}

/// Memory operation tag — used for tracing and analysis.
///
/// Classifies a single memory access into one of three actions that correspond
/// to the TLA+ actions available to each core.
///
/// # TLA+ Correspondence
/// Represents the three actions in `SimulatedMemory.tla`: `Read`, `Write`, `Fence`.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Memory",
        link = "LEP-0005-laplace-interfaces-memory_model"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryOp {
    /// Read from a memory address (with local store-buffer forwarding).
    ///
    /// If the address has a pending buffered write on `core`, the buffer value
    /// is returned. Otherwise, main memory is consulted.
    Read {
        /// Core performing the read.
        core: CoreId,
        /// Address to read from.
        addr: Address,
    },

    /// Write to a memory address (placed in the core's store buffer).
    ///
    /// The write is appended to the store buffer and is not yet visible to other cores.
    Write {
        /// Core performing the write.
        core: CoreId,
        /// Address to write to.
        addr: Address,
        /// Value to write.
        val: Value,
    },

    /// Memory fence — flush the core's store buffer to main memory.
    ///
    /// Drains all pending entries in FIFO order, committing each write to main memory.
    Fence {
        /// Core issuing the fence.
        core: CoreId,
    },
}

impl fmt::Display for MemoryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemoryOp::Read { core, addr } => {
                write!(f, "Read(core={}, addr={})", core, addr)
            }
            MemoryOp::Write { core, addr, val } => {
                write!(f, "Write(core={}, addr={}, val={})", core, addr, val)
            }
            MemoryOp::Fence { core } => {
                write!(f, "Fence(core={})", core)
            }
        }
    }
}

/// Memory consistency model that governs store buffer drain semantics.
///
/// Determines when buffered writes become visible to other cores.
///
/// # TLA+ Correspondence
/// Different models impose different constraints on operation reordering in
/// `SimulatedMemory.tla`.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Memory",
        link = "LEP-0005-laplace-interfaces-memory_model"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsistencyModel {
    /// Sequential Consistency (SC) — no reordering; writes are immediately visible.
    SequentiallyConsistent,

    /// Relaxed Consistency — writes are buffered and may be delayed.
    ///
    /// Models weak memory architectures such as ARM or PowerPC.
    Relaxed,
}

/// Configuration for a memory simulation instance.
///
/// Passed to backend constructors and [`super::traits::ConfigurableBackend::with_config`]
/// to control the shape of the simulated memory system.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Memory",
        link = "LEP-0005-laplace-interfaces-memory_model"
    )
)]
#[derive(Debug, Clone)]
pub struct MemoryConfig {
    /// Number of CPU cores in the system.
    ///
    /// In verification mode this is capped at 2 for tractability.
    pub num_cores: usize,

    /// Maximum store buffer entries per core.
    ///
    /// In verification mode this is capped at 2 to keep the state space manageable.
    pub max_buffer_size: usize,

    /// The consistency model to simulate.
    pub consistency_model: ConsistencyModel,

    /// Initial addressable memory size.
    ///
    /// In verification mode this is capped at 4 addresses.
    pub initial_size: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            num_cores: 2,
            max_buffer_size: 2,
            consistency_model: ConsistencyModel::Relaxed,
            initial_size: 1024,
        }
    }
}
