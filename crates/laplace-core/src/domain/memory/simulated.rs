//! Simulated memory system with pluggable backends.
//!
//! This module provides the high-level `SimulatedMemory` abstraction that combines
//! a memory backend with a virtual clock for event scheduling. It implements the
//! core memory operations (read, write, fence, flush) with direct TLA+ correspondence.
//!
//! **Principle**: Fractal Integrity — SimulatedMemory delegates to backend and clock
//! without maintaining internal state beyond configuration.

use super::traits::MemoryBackend;
use super::types::{Address, CoreId, MemoryConfig, StoreEntry, Value};
use crate::domain::time::{ClockBackend, VirtualClock};

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Simulated memory system combining backend storage and virtual clock.
///
/// The `SimulatedMemory` struct brings together a memory backend implementation
/// and a virtual clock for event scheduling. This allows the memory system to
/// record when operations occur and schedule dependent events (like store buffer
/// flushes) without coupling to any external framework.
///
/// # Type Parameters
///
/// - `MB`: Memory backend implementation (ProductionBackend or VerificationBackend)
/// - `CB`: Clock backend for event scheduling (used internally by VirtualClock)
///
/// # Architecture
///
/// ```text
/// ┌─────────────────────────────────────┐
/// │       SimulatedMemory<MB, CB>       │
/// ├─────────────────────────────────────┤
/// │ • backend: MB                       │
/// │ • clock: VirtualClock<CB>           │
/// │ • config: MemoryConfig              │
/// └─────────────────────────────────────┘
/// ```
///
/// The memory backend handles storage (main memory and store buffers), while
/// the virtual clock manages event scheduling for operations like deferred
/// store buffer flushes.
///
/// # TLA+ Correspondence
///
/// ```tla
/// VARIABLES mainMemory, storeBuffers
/// ```
///
/// The SimulatedMemory module directly corresponds to the TLA+ SimulatedMemory.tla
/// specification, with each operation having explicit TLA+ semantics.
///
/// # Example
///
/// ```ignore
/// use laplace_core::domain::memory::{ProductionBackend, MemoryConfig, SimulatedMemory};
/// use laplace_core::domain::time::{VirtualClock, ProdClockBackend};
///
/// let backend = ProductionBackend::new(4, 256);
/// let clock = VirtualClock::new();
/// let config = MemoryConfig::default();
///
/// let mut memory = SimulatedMemory::new(backend, clock, config);
/// memory.write(0, Address(0x1000), 42)?;
/// let value = memory.read(0, Address(0x1000)); // May be buffered or from main memory
/// ```
pub struct SimulatedMemory<MB: MemoryBackend, CB: ClockBackend> {
    /// Memory backend storing main memory and store buffers.
    backend: MB,
    /// Virtual clock for event scheduling.
    clock: VirtualClock<CB>,
    /// Memory system configuration.
    config: MemoryConfig,
}

impl<MB: MemoryBackend, CB: ClockBackend> SimulatedMemory<MB, CB> {
    /// Create new simulated memory system.
    ///
    /// Initializes both the memory backend and the virtual clock, establishing
    /// the foundation for memory operations and event scheduling.
    ///
    /// # Arguments
    ///
    /// * `backend` - The memory backend (ProductionBackend or VerificationBackend)
    /// * `clock` - Virtual clock for managing timed events
    /// * `config` - Configuration parameters for the memory system
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// Init ==
    ///     /\ mainMemory = [a \in Addresses |-> 0]
    ///     /\ storeBuffers = [c \in Cores |-> <<>>]
    /// ```
    pub fn new(backend: MB, clock: VirtualClock<CB>, config: MemoryConfig) -> Self {
        Self {
            backend,
            clock,
            config,
        }
    }

    /// Read value from address with store buffer forwarding.
    ///
    /// Implements load forwarding semantics: if this core has a pending write
    /// to the address in its store buffer, return the buffered value. Otherwise,
    /// read from main memory.
    ///
    /// # Algorithm
    ///
    /// 1. Check store buffer first (local forwarding)
    /// 2. If found, return buffered value
    /// 3. Otherwise, read from main memory
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// ReadValue(core, addr) ==
    ///     LET bufVal == BufferLookup(core, addr)
    ///     IN  IF bufVal /= "NONE" THEN bufVal ELSE mainMemory[addr]
    /// ```
    ///
    /// # Arguments
    ///
    /// * `core` - The core performing the read
    /// * `addr` - The memory address to read from
    ///
    /// # Returns
    ///
    /// The value at the address (either from buffer or main memory)
    pub fn read(&self, core: CoreId, addr: Address) -> Value {
        // Local forwarding: check this core's buffer first
        if let Some(val) = self.backend.buffer_lookup(core, addr) {
            return val;
        }

        // Fallback to main memory
        self.backend.read_main(addr)
    }

    /// Write value to address (buffered).
    ///
    /// Stores the write in this core's store buffer and schedules a deferred
    /// sync event to eventually flush it to main memory. This implements the
    /// store-buffered memory model where writes are not immediately visible
    /// to other cores.
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// Write(core, addr, val) ==
    ///     /\ Len(storeBuffers[core]) < MaxBufferSize
    ///     /\ LET newLamport == lamportClock + 1
    ///        IN  /\ storeBuffers' = [storeBuffers EXCEPT
    ///                ![core] = Append(@, [addr |-> addr, val |-> val])]
    ///            /\ eventQueue' = eventQueue \cup {CreateEvent("WRITE_SYNC", ...)}
    ///            /\ lamportClock' = newLamport
    /// ```
    ///
    /// # Arguments
    ///
    /// * `core` - The core performing the write
    /// * `addr` - The memory address to write to
    /// * `val` - The value to write
    ///
    /// # Returns
    ///
    /// - `Ok(())` if the write was successfully buffered
    /// - `Err(&'static str)` if the store buffer is full
    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(
            layer = "20_Core_Memory",
            link = "LEP-0002-laplace-core-memory_smt_optimization"
        )
    )]
    pub fn write(&mut self, core: CoreId, addr: Address, val: Value) -> Result<(), &'static str> {
        // Add to store buffer
        let entry = StoreEntry::new(addr, val);
        self.backend.buffer_push(core, entry)?;

        // Schedule sync event (flush to main memory on next tick)
        let delay_ns = 1;
        self.clock
            .schedule_write_sync(delay_ns, core, addr, val.as_u64());

        Ok(())
    }

    /// Flush one entry from store buffer to main memory.
    ///
    /// Pops the oldest entry from the specified core's store buffer and writes
    /// it to main memory. This operation is called as part of fence execution
    /// or store buffer eviction when the buffer becomes full.
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// FlushOneEntry(core) ==
    ///     LET entry == Head(storeBuffers[core])
    ///     IN  /\ mainMemory' = [mainMemory EXCEPT ![entry.addr] = entry.val]
    ///         /\ storeBuffers' = [storeBuffers EXCEPT ![core] = Tail(@)]
    /// ```
    ///
    /// # Arguments
    ///
    /// * `core` - The core whose store buffer to flush from
    ///
    /// # Returns
    ///
    /// - `Ok(())` if an entry was successfully flushed
    /// - `Err(&'static str)` if the store buffer is empty
    pub fn flush_one(&mut self, core: CoreId) -> Result<(), &'static str> {
        let entry = self.backend.buffer_pop(core).ok_or("Store buffer empty")?;

        // Write to main memory
        self.backend.write_main(entry.addr, entry.val);

        Ok(())
    }

    /// Memory fence - schedule flush of all pending writes.
    ///
    /// A fence operation ensures that all pending writes in the specified core's
    /// store buffer are eventually written to main memory. This implements the
    /// fence semantics required for correct concurrent memory access.
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// Fence(core) ==
    ///     /\ storeBuffers[core] /= <<>>
    ///     /\ eventQueue' = eventQueue \cup {CreateEvent("FENCE", ...)}
    /// ```
    ///
    /// # Arguments
    ///
    /// * `core` - The core performing the fence
    ///
    /// # Returns
    ///
    /// - `Ok(())` if the fence was successfully scheduled (or buffer already empty)
    /// - `Err(&'static str)` on operational failure
    pub fn fence(&mut self, core: CoreId) -> Result<(), &'static str> {
        if self.backend.is_buffer_empty(core) {
            return Ok(());
        }

        // Schedule fence event
        self.clock.schedule_fence(1, core);

        Ok(())
    }

    /// Get current value from main memory, bypassing store buffers.
    ///
    /// Directly reads from main memory without checking the store buffer.
    /// This is useful for observing the globally visible memory state,
    /// which may differ from what a load would see (due to store forwarding).
    ///
    /// # Arguments
    ///
    /// * `addr` - The memory address to read from
    ///
    /// # Returns
    ///
    /// The value currently in main memory at the address
    pub fn read_main_memory(&self, addr: Address) -> Value {
        self.backend.read_main(addr)
    }

    /// Get the number of pending entries in a core's store buffer.
    ///
    /// Returns the count of entries currently in the specified core's store buffer
    /// that have not yet been flushed to main memory.
    ///
    /// # Arguments
    ///
    /// * `core` - The core whose buffer to inspect
    ///
    /// # Returns
    ///
    /// The number of entries in the store buffer
    pub fn get_buffer_len(&self, core: CoreId) -> usize {
        self.backend.buffer_len(core)
    }

    /// Check if all store buffers are empty.
    ///
    /// Returns true if there are no pending writes in any core's store buffer.
    /// This is useful for determining when the system has reached a quiescent state.
    ///
    /// # Returns
    ///
    /// `true` if all store buffers are empty, `false` otherwise
    pub fn all_buffers_empty(&self) -> bool {
        (0..self.backend.num_cores()).all(|core_id| self.backend.is_buffer_empty(core_id.into()))
    }

    /// Get a reference to the memory configuration.
    ///
    /// # Returns
    ///
    /// Reference to the memory system configuration
    pub fn config(&self) -> &MemoryConfig {
        &self.config
    }

    /// Get a reference to the virtual clock.
    ///
    /// Provides read-only access to the clock for inspecting scheduled events
    /// and current time state.
    ///
    /// # Returns
    ///
    /// Reference to the virtual clock
    pub fn clock(&self) -> &VirtualClock<CB> {
        &self.clock
    }

    /// Get mutable access to the virtual clock.
    ///
    /// Provides mutable access for event processing and time advancement.
    /// This is typically used internally by the simulation engine.
    ///
    /// # Returns
    ///
    /// Mutable reference to the virtual clock
    pub fn clock_mut(&mut self) -> &mut VirtualClock<CB> {
        &mut self.clock
    }

    /// Get a reference to the memory backend.
    ///
    /// Provides direct access to the underlying storage backend for advanced
    /// inspection or specialized operations.
    ///
    /// # Returns
    ///
    /// Reference to the memory backend
    pub fn backend(&self) -> &MB {
        &self.backend
    }

    /// Get mutable access to the memory backend.
    ///
    /// Provides mutable access for backend-specific operations.
    ///
    /// # Returns
    ///
    /// Mutable reference to the memory backend
    pub fn backend_mut(&mut self) -> &mut MB {
        &mut self.backend
    }

    /// Reset the memory system to initial state.
    ///
    /// Clears all memory contents (both main memory and store buffers) and
    /// resets the virtual clock. This is typically called between simulation runs.
    pub fn reset(&mut self) {
        self.backend.clear_all();
        self.clock.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simulated_memory_creation() {
        // 이름 충돌 방지를 위해 별칭(as) 사용
        use crate::domain::memory::VerificationBackend as MemBackend;
        use crate::domain::time::ProductionBackend as ClockBackend;
        use crate::domain::time::VirtualClock;

        let mem_backend = MemBackend::new();

        let clock_backend = ClockBackend::new();
        let clock = VirtualClock::new(clock_backend);

        let config = MemoryConfig::default();

        // 3. 조립
        let _memory = SimulatedMemory::new(mem_backend, clock, config);
        // Successfully created
    }
}
