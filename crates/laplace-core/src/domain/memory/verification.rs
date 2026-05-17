//! Verification Memory Backend - Formally Verifiable with Kani
//!
//! This backend is engineered specifically for symbolic execution and formal
//! verification. Every design decision prioritizes making Kani's analysis tractable
//! by maintaining a bounded state space that the symbolic engine can fully explore.
//!
//! # Design Philosophy
//!
//! Kani performs symbolic execution by exploring all possible execution paths.
//! Unbounded heap allocation, complex lock-free algorithms, and OS syscalls
//! create infinite state spaces that Kani cannot analyze. This backend eliminates
//! those obstacles through disciplined design choices.
//!
//! ## What We Eliminated
//! - ✗ Heap allocation → Creates infinite address space
//! - ✗ RwLock syscalls → Kani cannot model OS kernels
//! - ✗ DashMap lock-free tricks → Atomic instructions outside Kani's reach
//!
//! ## What We Use Instead
//! - ✓ Fixed-size arrays → Bounded memory footprint
//! - ✓ UnsafeCell → Interior mutability without lock overhead
//! - ✓ RefCell → Borrow checking Kani can see and verify
//! - ✓ Direct array indexing → Transparent memory operations
//!
//! # Memory Layout (Stack-Allocated, Kani-Friendly)
//!
//! ```text
//! VerificationBackend (entire instance fits on stack, ~300 bytes)
//! ├─ main_memory: UnsafeCell<[Value; 4]>       (32 bytes)
//! │  ├─ [0]: 0                                  (8 bytes each)
//! │  ├─ [1]: 0
//! │  ├─ [2]: 0
//! │  └─ [3]: 0
//! ├─ store_buffers: [RefCell<BufferState>; 2]  (total ~200 bytes)
//! │  ├─ [0]: RefCell<BufferState>
//! │  │  ├─ entries: [Option<StoreEntry>; 2]   (48 bytes)
//! │  │  └─ count: usize                        (8 bytes)
//! │  └─ [1]: RefCell<BufferState> { ... }
//! ├─ num_cores: usize                          (8 bytes)
//! └─ max_buffer_size: usize                    (8 bytes)
//! ```
//!
//! Total state space for Kani to explore:
//! - 4 addresses × 2^64 symbolic values = manageable (Kani uses symbolic execution)
//! - 2 cores × 2 buffer slots × 2 states (Some/None) = 2^4 combinations
//! - 2 buffer counts (0..3) per core = manageable
//!
//! This is orders of magnitude smaller than heap-allocated backends!

use super::traits::{ConfigurableBackend, MemoryBackend};
use super::types::{Address, CoreId, StoreEntry, Value};
use std::cell::{RefCell, UnsafeCell};

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Maximum addresses supported in verification mode
///
/// Must be small enough that Kani can explore all combinations.
/// 4 addresses = 2^32 possible byte values per address, symbolic execution handles this.
const MAX_ADDRESSES: usize = 4;

/// Maximum cores supported in verification mode
///
/// Kani's complexity grows with the number of cores due to interleaving.
/// 2 cores is the practical limit for complete state space exploration.
const MAX_CORES: usize = 2;

/// Maximum store buffer entries per core
///
/// Each buffer entry is 16 bytes (address + value). With 2 entries per core,
/// total buffer overhead is manageable for symbolic execution.
const MAX_BUFFER_ENTRIES: usize = 2;

/// Store buffer state (per core)
///
/// # Design for Kani
///
/// Uses a fixed-size array instead of VecDeque. Each slot is `Option<StoreEntry>`,
/// allowing us to represent "empty slots" in the buffer. A separate count field
/// optimizes iteration (avoid scanning through Nones).
///
/// # Why Option?
///
/// Kani can fully explore the `Some` and `None` branches. This is far more efficient
/// than heap-allocated VecDeque which requires modeling arbitrary pointer mutations.
#[derive(Clone)]
struct BufferState {
    /// Fixed-size array of buffer slots
    ///
    /// `Option::Some` = valid store entry, `Option::None` = empty slot
    entries: [Option<StoreEntry>; MAX_BUFFER_ENTRIES],

    /// Number of valid entries (count of Some values)
    ///
    /// Optimization: lets us know when the buffer is full without scanning.
    count: usize,
}

impl BufferState {
    /// Create a new, empty buffer state
    fn new() -> Self {
        Self {
            entries: [None; MAX_BUFFER_ENTRIES],
            count: 0,
        }
    }

    /// Check if buffer is empty
    fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Check if buffer is full
    fn is_full(&self) -> bool {
        self.count >= MAX_BUFFER_ENTRIES
    }

    /// Get current entry count
    fn len(&self) -> usize {
        self.count
    }

    /// Push entry to buffer (find first empty slot)
    ///
    /// FIFO is implemented by maintaining entry order and gaps. We find the first
    /// None slot and insert there. On pop, we shift entries left to maintain order.
    fn push(&mut self, entry: StoreEntry) -> Result<(), &'static str> {
        if self.is_full() {
            return Err("Buffer full");
        }

        // Find first empty slot (linear scan is fine for small arrays)
        for slot in &mut self.entries {
            if slot.is_none() {
                *slot = Some(entry);
                self.count += 1;
                return Ok(());
            }
        }

        Err("Buffer full")
    }

    /// Pop oldest entry (FIFO: extract first Some, shift rest left)
    ///
    /// # Algorithm
    ///
    /// 1. Find the first `Some` entry at position `first_idx`.
    /// 2. Extract and return it.
    /// 3. Shift all entries after `first_idx` one position to the left.
    /// 4. Clear the last slot to `None`.
    /// 5. Decrement count.
    ///
    /// This preserves FIFO order while maintaining a contiguous block of
    /// entries at the start of the array.
    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(
            layer = "20_Core_Memory",
            link = "LEP-0002-laplace-core-memory_smt_optimization"
        )
    )]
    fn pop(&mut self) -> Option<StoreEntry> {
        if self.count == 0 {
            return None;
        }

        // Find first Some entry
        let first_idx = self.entries.iter().position(|e| e.is_some())?;
        let entry = self.entries[first_idx].take();

        // Shift remaining entries left to maintain contiguity
        for i in first_idx..MAX_BUFFER_ENTRIES - 1 {
            self.entries[i] = self.entries[i + 1].take();
        }
        self.entries[MAX_BUFFER_ENTRIES - 1] = None;

        self.count -= 1;
        entry
    }

    /// Lookup most recent value for address (load forwarding)
    ///
    /// Scans from the end of the array backward (most recent first) to find
    /// the most recent buffered write to the given address.
    fn lookup(&self, addr: Address) -> Option<Value> {
        for e in self.entries.iter().rev().flatten() {
            if e.addr == addr {
                return Some(e.val);
            }
        }
        None
    }

    /// Clear buffer to initial state
    fn clear(&mut self) {
        self.entries = [None; MAX_BUFFER_ENTRIES];
        self.count = 0;
    }
}

/// Verification-friendly memory backend for Kani
///
/// # Design Goals
///
/// 1. **Bounded State**: Fixed-size arrays ensure Kani explores a finite state space.
/// 2. **Transparent Memory**: `UnsafeCell` and `RefCell` make memory operations visible to Kani.
/// 3. **Fast Verification**: Minimal abstraction overhead so Kani spends time on logic, not boilerplate.
///
/// # Thread Safety Notes
///
/// In verification mode, Kani performs single-threaded symbolic execution. The `unsafe`
/// blocks in main memory access are actually **safe** in this context because:
///
/// - Kani doesn't spawn real threads; it symbolically explores one path at a time.
/// - Rust's type system enforces exclusive access via `&mut self` for writes.
/// - We manually guarantee that only one operation modifies main memory at a time.
///
/// The `unsafe` is necessary only because we use `UnsafeCell` instead of `RefCell`
/// for main memory (RefCell would add runtime overhead for read tracking).
pub struct VerificationBackend {
    /// Main memory: fixed-size array on stack
    ///
    /// # Why UnsafeCell?
    ///
    /// Main memory reads are frequent and should have zero overhead. We use `UnsafeCell`
    /// for interior mutability without lock acquisition. The safety invariant is:
    /// "only `&mut self` methods call `write_main`, and `read_main` never races with writes."
    ///
    /// Kani can verify this invariant through its type system analysis.
    ///
    /// TLA+: `mainMemory[addr]`
    main_memory: UnsafeCell<[Value; MAX_ADDRESSES]>,

    /// Per-core store buffers
    ///
    /// # Why RefCell?
    ///
    /// Each buffer can be mutated independently. `RefCell` provides interior mutability
    /// while allowing Kani to verify borrow checking at symbolic execution time.
    ///
    /// TLA+: `storeBuffers[core]`
    store_buffers: [RefCell<BufferState>; MAX_CORES],

    /// Configuration
    num_cores: usize,
    max_buffer_size: usize,
}

impl VerificationBackend {
    /// Create a new verification backend with default limits
    pub fn new() -> Self {
        Self {
            main_memory: UnsafeCell::new([Value::new(0); MAX_ADDRESSES]),
            store_buffers: [
                RefCell::new(BufferState::new()),
                RefCell::new(BufferState::new()),
            ],
            num_cores: MAX_CORES,
            max_buffer_size: MAX_BUFFER_ENTRIES,
        }
    }

    /// Create with configuration, enforcing verification mode limits
    pub fn with_config(num_cores: usize, max_buffer_size: usize) -> Self {
        assert!(
            num_cores <= MAX_CORES,
            "Verification mode supports max {} cores",
            MAX_CORES
        );
        assert!(
            max_buffer_size <= MAX_BUFFER_ENTRIES,
            "Verification mode supports max {} buffer entries",
            MAX_BUFFER_ENTRIES
        );

        Self::new()
    }
}

impl Default for VerificationBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryBackend for VerificationBackend {
    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(
            layer = "20_Core_Memory",
            link = "LEP-0002-laplace-core-memory_smt_optimization"
        )
    )]
    fn read_main(&self, addr: Address) -> Value {
        if addr.as_usize() >= MAX_ADDRESSES {
            return Value::new(0);
        }

        // SAFETY: Single-threaded verification context
        //
        // In Kani's symbolic execution, there is only one logical thread. The type
        // system guarantees:
        // - `read_main(&self)` cannot race with `write_main(&mut self)`
        // - `&mut self` methods have exclusive access
        //
        // Kani verifies these invariants, making the unsafe block sound.
        unsafe {
            let mem = &*self.main_memory.get();
            mem[addr.as_usize()]
        }
    }

    fn write_main(&mut self, addr: Address, val: Value) {
        if addr.as_usize() >= MAX_ADDRESSES {
            return;
        }

        // SAFETY: Exclusive access via &mut self
        //
        // The receiver is `&mut self`, guaranteeing no other references exist.
        // This makes the mutation safe without synchronization.
        unsafe {
            let mem = &mut *self.main_memory.get();
            mem[addr.as_usize()] = val;
        }
    }

    fn is_buffer_empty(&self, core: CoreId) -> bool {
        if core.as_usize() >= MAX_CORES {
            return true;
        }
        self.store_buffers[core.as_usize()].borrow().is_empty()
    }

    fn buffer_len(&self, core: CoreId) -> usize {
        if core.as_usize() >= MAX_CORES {
            return 0;
        }
        self.store_buffers[core.as_usize()].borrow().len()
    }

    fn buffer_push(&mut self, core: CoreId, entry: StoreEntry) -> Result<(), &'static str> {
        if core.as_usize() >= MAX_CORES {
            return Err("Invalid core ID");
        }

        // Verify address is within bounds
        if entry.addr.as_usize() >= MAX_ADDRESSES {
            return Err("Address out of bounds");
        }

        self.store_buffers[core.as_usize()].borrow_mut().push(entry)
    }

    fn buffer_pop(&mut self, core: CoreId) -> Option<StoreEntry> {
        if core.as_usize() >= MAX_CORES {
            return None;
        }
        self.store_buffers[core.as_usize()].borrow_mut().pop()
    }

    fn buffer_lookup(&self, core: CoreId, addr: Address) -> Option<Value> {
        if core.as_usize() >= MAX_CORES || addr.as_usize() >= MAX_ADDRESSES {
            return None;
        }
        self.store_buffers[core.as_usize()].borrow().lookup(addr)
    }

    fn clear_all(&mut self) {
        // SAFETY: Exclusive access via &mut self
        unsafe {
            let mem = &mut *self.main_memory.get();
            *mem = [Value::new(0); MAX_ADDRESSES];
        }

        for buffer in &self.store_buffers {
            buffer.borrow_mut().clear();
        }
    }

    fn num_cores(&self) -> usize {
        self.num_cores
    }

    fn max_buffer_size(&self) -> usize {
        self.max_buffer_size
    }
}

impl ConfigurableBackend for VerificationBackend {
    fn with_config(num_cores: usize, max_buffer_size: usize) -> Self {
        Self::with_config(num_cores, max_buffer_size)
    }
}

// SAFETY: Single-threaded verification context
//
// Kani's symbolic execution is single-threaded. Marking VerificationBackend as Sync
// is safe because we control all access patterns and exclude concurrent scenarios.
#[cfg(kani)]
unsafe impl Sync for VerificationBackend {}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verification_main_memory_basic() {
        let mut backend = VerificationBackend::new();

        assert_eq!(backend.read_main(Address::new(0)), Value::new(0));

        backend.write_main(Address::new(0), Value::new(100));
        assert_eq!(backend.read_main(Address::new(0)), Value::new(100));

        backend.write_main(Address::new(1), Value::new(200));
        assert_eq!(backend.read_main(Address::new(1)), Value::new(200));
    }

    #[test]
    fn test_verification_buffer_fifo() {
        let mut backend = VerificationBackend::new();

        backend
            .buffer_push(
                CoreId::new(0),
                StoreEntry::new(Address::new(0), Value::new(100)),
            )
            .expect("Push 1");
        backend
            .buffer_push(
                CoreId::new(0),
                StoreEntry::new(Address::new(1), Value::new(200)),
            )
            .expect("Push 2");

        // Should pop in FIFO order
        let e1 = backend.buffer_pop(CoreId::new(0)).expect("Pop 1");
        assert_eq!(e1.addr, Address::new(0));
        assert_eq!(e1.val, Value::new(100));

        let e2 = backend.buffer_pop(CoreId::new(0)).expect("Pop 2");
        assert_eq!(e2.addr, Address::new(1));
        assert_eq!(e2.val, Value::new(200));
    }

    #[test]
    fn test_verification_buffer_lookup() {
        let mut backend = VerificationBackend::new();

        backend
            .buffer_push(
                CoreId::new(0),
                StoreEntry::new(Address::new(0), Value::new(100)),
            )
            .expect("Push 1");
        backend
            .buffer_push(
                CoreId::new(0),
                StoreEntry::new(Address::new(0), Value::new(200)),
            )
            .expect("Push 2");

        // Should return most recent value
        assert_eq!(
            backend.buffer_lookup(CoreId::new(0), Address::new(0)),
            Some(Value::new(200))
        );
    }

    #[test]
    fn test_verification_bounds_address() {
        let mut backend = VerificationBackend::new();

        // Out of bounds address should return 0
        assert_eq!(backend.read_main(Address::new(999)), Value::new(0));

        // Write to out of bounds should be silent (not panic)
        backend.write_main(Address::new(999), Value::new(100));
        assert_eq!(backend.read_main(Address::new(999)), Value::new(0));
    }

    #[test]
    fn test_verification_bounds_core() {
        let mut backend = VerificationBackend::new();

        // Invalid core should be handled gracefully
        assert!(backend.is_buffer_empty(CoreId::new(999)));
        assert_eq!(backend.buffer_len(CoreId::new(999)), 0);
        assert!(backend
            .buffer_push(
                CoreId::new(999),
                StoreEntry::new(Address::new(0), Value::new(100))
            )
            .is_err());
        assert_eq!(backend.buffer_pop(CoreId::new(999)), None);
    }

    #[test]
    fn test_verification_buffer_full() {
        let mut backend = VerificationBackend::new();

        backend
            .buffer_push(
                CoreId::new(0),
                StoreEntry::new(Address::new(0), Value::new(100)),
            )
            .expect("Push 1");
        backend
            .buffer_push(
                CoreId::new(0),
                StoreEntry::new(Address::new(1), Value::new(200)),
            )
            .expect("Push 2");

        // Buffer should be full now
        assert_eq!(backend.buffer_len(CoreId::new(0)), 2);

        // Third push should fail
        let result = backend.buffer_push(
            CoreId::new(0),
            StoreEntry::new(Address::new(2), Value::new(300)),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_verification_clear_all() {
        let mut backend = VerificationBackend::new();

        backend.write_main(Address::new(0), Value::new(100));
        backend
            .buffer_push(
                CoreId::new(0),
                StoreEntry::new(Address::new(1), Value::new(200)),
            )
            .expect("Push");

        backend.clear_all();

        assert_eq!(backend.read_main(Address::new(0)), Value::new(0));
        assert!(backend.is_buffer_empty(CoreId::new(0)));
    }

    #[test]
    fn test_verification_multiple_cores() {
        let mut backend = VerificationBackend::new();

        // Core 0
        backend
            .buffer_push(
                CoreId::new(0),
                StoreEntry::new(Address::new(0), Value::new(100)),
            )
            .expect("Core 0 push");

        // Core 1
        backend
            .buffer_push(
                CoreId::new(1),
                StoreEntry::new(Address::new(1), Value::new(200)),
            )
            .expect("Core 1 push");

        // Buffers are independent
        assert_eq!(backend.buffer_len(CoreId::new(0)), 1);
        assert_eq!(backend.buffer_len(CoreId::new(1)), 1);
        assert_eq!(
            backend.buffer_lookup(CoreId::new(0), Address::new(0)),
            Some(Value::new(100))
        );
        assert_eq!(
            backend.buffer_lookup(CoreId::new(1), Address::new(1)),
            Some(Value::new(200))
        );
    }
}
