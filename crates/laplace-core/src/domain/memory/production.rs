//! Production Memory Backend
//!
//! Engineered for real-world concurrent workloads using lock-free concurrent
//! data structures and dynamic heap allocation. This backend prioritizes
//! performance and scalability for production scenarios.

use super::traits::{ConfigurableBackend, MemoryBackend};
use super::types::{Address, CoreId, StoreEntry, Value};
use dashmap::DashMap;
use parking_lot::RwLock;
use std::collections::VecDeque;

/// Store buffer for a single core
///
/// # Design
///
/// Maintains a FIFO queue of pending write operations. Each core has its own
/// buffer protected by a read-write lock, enabling concurrent readers while
/// serializing writers. This matches the TLA+ model where store buffers are
/// per-core state.
///
/// # TLA+ Correspondence
///
/// ```tla
/// storeBuffers: [Cores -> Seq(StoreEntry)]
/// ```
#[derive(Debug, Clone)]
struct StoreBuffer {
    /// Core identifier (for debugging)
    #[allow(dead_code)]
    core_id: CoreId,

    /// FIFO queue of buffered write entries
    entries: VecDeque<StoreEntry>,

    /// Maximum buffer capacity
    max_size: usize,
}

impl StoreBuffer {
    /// Create a new store buffer for a core
    ///
    /// # Arguments
    ///
    /// - `core_id`: The identifier of the core this buffer belongs to.
    /// - `max_size`: The maximum number of entries allowed in the buffer.
    fn new(core_id: CoreId, max_size: usize) -> Self {
        Self {
            core_id,
            entries: VecDeque::with_capacity(max_size),
            max_size,
        }
    }

    /// Check if the buffer is empty
    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Check if the buffer is full
    fn is_full(&self) -> bool {
        self.entries.len() >= self.max_size
    }

    /// Get the current number of entries
    fn len(&self) -> usize {
        self.entries.len()
    }

    /// Add an entry to the buffer (FIFO tail)
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// storeBuffers' = [storeBuffers EXCEPT ![core] = Append(@, entry)]
    /// ```
    fn push(&mut self, entry: StoreEntry) -> Result<(), &'static str> {
        if self.is_full() {
            return Err("Store buffer full");
        }
        self.entries.push_back(entry);
        Ok(())
    }

    /// Remove and return the oldest entry (FIFO head)
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// storeBuffers' = [storeBuffers EXCEPT ![core] = Tail(@)]
    /// ```
    fn pop(&mut self) -> Option<StoreEntry> {
        self.entries.pop_front()
    }

    /// Lookup the most recent value for an address (load forwarding)
    ///
    /// Searches from newest to oldest to find the most recent write to the
    /// given address.
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// BufferLookup(core, addr) ==
    ///     LET buf == storeBuffers[core]
    ///         matching == {i \in 1..Len(buf) : buf[i].addr = addr}
    ///     IN  IF matching = {} THEN "NONE"
    ///         ELSE buf[CHOOSE x \in matching : \A y \in matching : x >= y].val
    /// ```
    fn lookup(&self, addr: Address) -> Option<Value> {
        self.entries
            .iter()
            .rev()
            .find(|entry| entry.addr == addr)
            .map(|entry| entry.val)
    }

    /// Clear all entries from the buffer
    fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Production-grade memory backend
///
/// # Design for Maximum Performance
///
/// This backend is engineered for real-world concurrent workloads and
/// emphasizes high throughput and low latency:
///
/// - **DashMap for main memory**: A lock-free concurrent hash map enabling
///   multiple threads to read and write simultaneously without coordination.
///
/// - **RwLock for per-core buffers**: Each core's store buffer is protected
///   by a read-write lock, allowing many readers but exclusive writers.
///
/// - **Dynamic heap allocation**: Main memory can grow to accommodate arbitrary
///   address ranges. Store buffers grow with demand.
///
/// # Memory Layout
///
/// ```text
/// ProductionBackend
/// ├─ main_memory: DashMap<Address, Value>  (concurrent hashmap)
/// ├─ store_buffers: Vec<RwLock<StoreBuffer>>
/// │  ├─ [0]: RwLock<StoreBuffer>  (Core 0's buffer)
/// │  ├─ [1]: RwLock<StoreBuffer>  (Core 1's buffer)
/// │  └─ ...
/// ├─ num_cores: usize
/// └─ max_buffer_size: usize
/// ```
///
/// # Thread Safety
///
/// - `DashMap` allows concurrent reads and writes without external locking.
/// - `RwLock` protects each store buffer individually, enabling parallel
///   buffer operations on different cores.
/// - Multiple cores can write to different buffers simultaneously.
///
/// # TLA+ Correspondence
///
/// ```tla
/// VARIABLES mainMemory, storeBuffers
/// ```
pub struct ProductionBackend {
    /// Main memory: concurrent hashmap for O(1) lookups and inserts
    ///
    /// TLA+: `mainMemory[addr]`
    main_memory: DashMap<Address, Value>,

    /// Per-core store buffers (FIFO queues)
    ///
    /// TLA+: `storeBuffers[core]`
    store_buffers: Vec<RwLock<StoreBuffer>>,

    /// Configuration: number of cores
    num_cores: usize,

    /// Configuration: maximum buffer size per core
    max_buffer_size: usize,
}

impl ProductionBackend {
    /// Create a new production backend
    ///
    /// # Arguments
    ///
    /// - `num_cores`: Number of cores to simulate.
    /// - `max_buffer_size`: Maximum store buffer capacity per core.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let backend = ProductionBackend::new(4, 256);
    /// ```
    pub fn new(num_cores: usize, max_buffer_size: usize) -> Self {
        let store_buffers = (0..num_cores)
            .map(|core_id| RwLock::new(StoreBuffer::new(CoreId::new(core_id), max_buffer_size)))
            .collect();

        Self {
            main_memory: DashMap::new(),
            store_buffers,
            num_cores,
            max_buffer_size,
        }
    }
}

impl MemoryBackend for ProductionBackend {
    fn read_main(&self, addr: Address) -> Value {
        // DashMap::get returns a Ref smart pointer; we dereference to get the value
        self.main_memory
            .get(&addr)
            .map(|r| *r)
            .unwrap_or(Value::new(0))
    }

    fn write_main(&mut self, addr: Address, val: Value) {
        // DashMap::insert is atomic and lock-free
        self.main_memory.insert(addr, val);
    }

    fn is_buffer_empty(&self, core: CoreId) -> bool {
        if core.as_usize() >= self.num_cores {
            return true;
        }
        self.store_buffers[core.as_usize()].read().is_empty()
    }

    fn buffer_len(&self, core: CoreId) -> usize {
        if core.as_usize() >= self.num_cores {
            return 0;
        }
        self.store_buffers[core.as_usize()].read().len()
    }

    fn buffer_push(&mut self, core: CoreId, entry: StoreEntry) -> Result<(), &'static str> {
        if core.as_usize() >= self.num_cores {
            return Err("Invalid core ID");
        }

        // Acquire write lock on this core's buffer
        self.store_buffers[core.as_usize()].write().push(entry)
    }

    fn buffer_pop(&mut self, core: CoreId) -> Option<StoreEntry> {
        if core.as_usize() >= self.num_cores {
            return None;
        }

        // FIFO: pop from head
        self.store_buffers[core.as_usize()].write().pop()
    }

    fn buffer_lookup(&self, core: CoreId, addr: Address) -> Option<Value> {
        if core.as_usize() >= self.num_cores {
            return None;
        }

        // Search from newest to oldest for load forwarding
        self.store_buffers[core.as_usize()].read().lookup(addr)
    }

    fn clear_all(&mut self) {
        self.main_memory.clear();
        for buffer in &self.store_buffers {
            buffer.write().clear();
        }
    }

    fn num_cores(&self) -> usize {
        self.num_cores
    }

    fn max_buffer_size(&self) -> usize {
        self.max_buffer_size
    }
}

impl ConfigurableBackend for ProductionBackend {
    fn with_config(num_cores: usize, max_buffer_size: usize) -> Self {
        Self::new(num_cores, max_buffer_size)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_production_main_memory_basic() {
        let mut backend = ProductionBackend::new(2, 4);

        // Initially zero
        assert_eq!(backend.read_main(Address::new(42)), Value::new(0));

        // Write and read
        backend.write_main(Address::new(42), Value::new(100));
        assert_eq!(backend.read_main(Address::new(42)), Value::new(100));

        // Different addresses
        backend.write_main(Address::new(99), Value::new(200));
        assert_eq!(backend.read_main(Address::new(99)), Value::new(200));
        assert_eq!(backend.read_main(Address::new(42)), Value::new(100));
    }

    #[test]
    fn test_production_store_buffer_basic() {
        let mut backend = ProductionBackend::new(2, 4);

        // Buffer starts empty
        assert!(backend.is_buffer_empty(CoreId::new(0)));
        assert_eq!(backend.buffer_len(CoreId::new(0)), 0);

        // Push entry
        backend
            .buffer_push(
                CoreId::new(0),
                StoreEntry::new(Address::new(10), Value::new(100)),
            )
            .expect("Push failed");
        assert!(!backend.is_buffer_empty(CoreId::new(0)));
        assert_eq!(backend.buffer_len(CoreId::new(0)), 1);

        // Pop entry
        let entry = backend.buffer_pop(CoreId::new(0)).expect("Pop failed");
        assert_eq!(entry.addr, Address::new(10));
        assert_eq!(entry.val, Value::new(100));
        assert!(backend.is_buffer_empty(CoreId::new(0)));
    }

    #[test]
    fn test_production_store_buffer_fifo() {
        let mut backend = ProductionBackend::new(2, 4);

        backend
            .buffer_push(
                CoreId::new(0),
                StoreEntry::new(Address::new(10), Value::new(100)),
            )
            .expect("Push 1");
        backend
            .buffer_push(
                CoreId::new(0),
                StoreEntry::new(Address::new(20), Value::new(200)),
            )
            .expect("Push 2");
        backend
            .buffer_push(
                CoreId::new(0),
                StoreEntry::new(Address::new(30), Value::new(300)),
            )
            .expect("Push 3");

        // Should pop in FIFO order
        let e1 = backend.buffer_pop(CoreId::new(0)).expect("Pop 1");
        assert_eq!(e1.addr, Address::new(10));

        let e2 = backend.buffer_pop(CoreId::new(0)).expect("Pop 2");
        assert_eq!(e2.addr, Address::new(20));

        let e3 = backend.buffer_pop(CoreId::new(0)).expect("Pop 3");
        assert_eq!(e3.addr, Address::new(30));
    }

    #[test]
    fn test_production_buffer_lookup() {
        let mut backend = ProductionBackend::new(2, 4);

        backend
            .buffer_push(
                CoreId::new(0),
                StoreEntry::new(Address::new(10), Value::new(100)),
            )
            .expect("Push 1");
        backend
            .buffer_push(
                CoreId::new(0),
                StoreEntry::new(Address::new(20), Value::new(200)),
            )
            .expect("Push 2");

        backend
            .buffer_push(
                CoreId::new(0),
                StoreEntry::new(Address::new(10), Value::new(300)),
            )
            .expect("Push 3 (overwrite addr 10)");

        // Should return most recent value for addr 10
        assert_eq!(
            backend.buffer_lookup(CoreId::new(0), Address::new(10)),
            Some(Value::new(300))
        );
        assert_eq!(
            backend.buffer_lookup(CoreId::new(0), Address::new(20)),
            Some(Value::new(200))
        );
        assert_eq!(
            backend.buffer_lookup(CoreId::new(0), Address::new(99)),
            None
        );
    }

    #[test]
    fn test_production_buffer_full() {
        let mut backend = ProductionBackend::new(2, 2);

        backend
            .buffer_push(
                CoreId::new(0),
                StoreEntry::new(Address::new(10), Value::new(100)),
            )
            .expect("Push 1");
        backend
            .buffer_push(
                CoreId::new(0),
                StoreEntry::new(Address::new(20), Value::new(200)),
            )
            .expect("Push 2");

        // Buffer is now full
        let result = backend.buffer_push(
            CoreId::new(0),
            StoreEntry::new(Address::new(30), Value::new(300)),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_production_multiple_cores() {
        let mut backend = ProductionBackend::new(2, 4);

        // Core 0 buffer
        backend
            .buffer_push(
                CoreId::new(0),
                StoreEntry::new(Address::new(10), Value::new(100)),
            )
            .expect("Core 0 push");
        assert_eq!(backend.buffer_len(CoreId::new(0)), 1);

        // Core 1 buffer
        backend
            .buffer_push(
                CoreId::new(1),
                StoreEntry::new(Address::new(20), Value::new(200)),
            )
            .expect("Core 1 push");
        assert_eq!(backend.buffer_len(CoreId::new(1)), 1);

        // Buffers are independent
        assert_eq!(
            backend.buffer_lookup(CoreId::new(0), Address::new(10)),
            Some(Value::new(100))
        );
        assert_eq!(
            backend.buffer_lookup(CoreId::new(0), Address::new(20)),
            None
        );
        assert_eq!(
            backend.buffer_lookup(CoreId::new(1), Address::new(20)),
            Some(Value::new(200))
        );
        assert_eq!(
            backend.buffer_lookup(CoreId::new(1), Address::new(10)),
            None
        );
    }

    #[test]
    fn test_production_clear_all() {
        let mut backend = ProductionBackend::new(2, 4);

        backend.write_main(Address::new(42), Value::new(100));
        backend
            .buffer_push(
                CoreId::new(0),
                StoreEntry::new(Address::new(10), Value::new(100)),
            )
            .expect("Push");

        backend.clear_all();

        assert_eq!(backend.read_main(Address::new(42)), Value::new(0));
        assert!(backend.is_buffer_empty(CoreId::new(0)));
    }

    #[test]
    fn test_production_invalid_core() {
        let mut backend = ProductionBackend::new(2, 4);

        // Invalid core IDs should be handled gracefully
        assert!(backend.is_buffer_empty(CoreId::new(999)));
        assert_eq!(backend.buffer_len(CoreId::new(999)), 0);
        assert!(backend
            .buffer_push(
                CoreId::new(999),
                StoreEntry::new(Address::new(10), Value::new(100))
            )
            .is_err());
        assert_eq!(backend.buffer_pop(CoreId::new(999)), None);
    }
}
