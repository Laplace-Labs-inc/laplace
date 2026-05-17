//! Memory Backend Trait Definitions
//!
//! Defines the abstract interfaces that all memory backend implementations must satisfy.
//! Consumers (e.g. `laplace-core`'s `SimulatedMemory`) program to these traits so that
//! production and verification backends are interchangeable at compile time.

use super::types::{Address, CoreId, StoreEntry, Value};

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Abstract interface for memory state management.
///
/// Implement this trait to provide a concrete memory backend for
/// `SimulatedMemory<B: MemoryBackend>`. The trait models two TLA+ state variables:
///
/// - **`mainMemory`**: global shared memory indexed by [`Address`].
/// - **`storeBuffers`**: per-core FIFO write queues, each containing [`StoreEntry`] records.
///
/// # Implementing
///
/// Both `ProductionBackend` (heap-allocated, concurrent) and `VerificationBackend`
/// (stack-allocated, fixed-size) implement this trait in `laplace-core`.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Memory",
        link = "LEP-0005-laplace-interfaces-memory_model"
    )
)]
pub trait MemoryBackend {
    /// Read directly from main memory, bypassing store buffers.
    ///
    /// - `addr`: The memory address to read from.
    ///
    /// Returns the value currently stored at `addr` in main memory (zero if unwritten).
    ///
    /// # TLA+ Correspondence
    /// `mainMemory[addr]`
    fn read_main(&self, addr: Address) -> Value;

    /// Write directly to main memory, bypassing store buffers.
    ///
    /// - `addr`: The memory address to write to.
    /// - `val`: The value to write.
    ///
    /// The write is immediately visible to all cores.
    ///
    /// # TLA+ Correspondence
    /// `mainMemory' = [mainMemory EXCEPT ![addr] = val]`
    fn write_main(&mut self, addr: Address, val: Value);

    /// Return `true` if the store buffer for `core` contains no pending writes.
    ///
    /// - `core`: The core identifier.
    ///
    /// # TLA+ Correspondence
    /// `storeBuffers[core] = <<>>`
    fn is_buffer_empty(&self, core: CoreId) -> bool;

    /// Return the number of pending write entries in `core`'s store buffer.
    ///
    /// - `core`: The core identifier.
    ///
    /// # TLA+ Correspondence
    /// `Len(storeBuffers[core])`
    fn buffer_len(&self, core: CoreId) -> usize;

    /// Append a write entry to the tail of `core`'s store buffer.
    ///
    /// - `core`: The core identifier.
    /// - `entry`: The [`StoreEntry`] (address + value) to enqueue.
    ///
    /// Returns `Ok(())` on success, or `Err(&'static str)` if the buffer is full
    /// or `core` is out of range.
    ///
    /// # TLA+ Correspondence
    /// `storeBuffers' = [storeBuffers EXCEPT ![core] = Append(@, entry)]`
    fn buffer_push(&mut self, core: CoreId, entry: StoreEntry) -> Result<(), &'static str>;

    /// Remove and return the oldest (head) entry from `core`'s store buffer.
    ///
    /// - `core`: The core identifier.
    ///
    /// Returns `Some(entry)` if the buffer is non-empty, `None` otherwise.
    ///
    /// # TLA+ Correspondence
    /// `LET entry == Head(storeBuffers[core]) IN storeBuffers' = [storeBuffers EXCEPT ![core] = Tail(@)]`
    fn buffer_pop(&mut self, core: CoreId) -> Option<StoreEntry>;

    /// Look up the most recent pending write for `addr` in `core`'s store buffer.
    ///
    /// - `core`: The core identifier.
    /// - `addr`: The memory address to search for.
    ///
    /// Returns `Some(value)` if a pending write exists for `addr`, `None` otherwise.
    /// When multiple entries match, the most recently added (last) one wins.
    ///
    /// # TLA+ Correspondence
    /// `BufferLookup(core, addr)` — local load forwarding.
    fn buffer_lookup(&self, core: CoreId, addr: Address) -> Option<Value>;

    /// Clear all store buffers and reset main memory to the zero state.
    ///
    /// Used for test initialization and scenario restart.
    fn clear_all(&mut self);

    /// Return the total number of cores this backend was configured for.
    fn num_cores(&self) -> usize;

    /// Return the maximum store buffer capacity per core.
    fn max_buffer_size(&self) -> usize;
}

/// Extension trait for backends that support parameterized construction.
///
/// Implement this alongside [`MemoryBackend`] to allow factories to create
/// backends with explicit `num_cores` and `max_buffer_size` settings.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Memory",
        link = "LEP-0005-laplace-interfaces-memory_model"
    )
)]
pub trait ConfigurableBackend: MemoryBackend + Sized {
    /// Create a backend configured for `num_cores` cores, each with a store buffer
    /// of at most `max_buffer_size` entries.
    ///
    /// - `num_cores`: Number of cores to simulate.
    /// - `max_buffer_size`: Maximum store buffer capacity per core.
    ///
    /// Returns a fully initialized backend instance.
    ///
    /// # Panics
    ///
    /// May panic if parameters exceed backend-specific limits (e.g. in verification
    /// mode where bounds are fixed for Kani tractability).
    fn with_config(num_cores: usize, max_buffer_size: usize) -> Self;
}
