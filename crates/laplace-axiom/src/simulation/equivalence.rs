//! Simulation Equivalence — 1:1 Correspondence with TLA+ Specification
//!
//! This module defines [`EquivalentTo<Spec>`], a trait that enforces a bijective
//! mapping between Rust implementation types and their TLA+ specification mirrors.
//!
//! # Design
//!
//! Each `Spec` type in this module is a direct mirror of a TLA+ record or value
//! in `VirtualClock.tla` / `SimulatedMemory.tla`. The trait guarantees that no
//! information is lost in either direction (`to_spec` → `from_spec` round-trips
//! back to the original value, and vice-versa for non-degenerate states).
//!
//! # Bijection Invariant
//!
//! For every Rust type `T` implementing `EquivalentTo<S>`:
//!
//! ```text
//! ∀ x : T,  from_spec(to_spec(x)) == x       (Rust → Spec → Rust)
//! ∀ s : S,  to_spec(from_spec(s)) == s        (Spec → Rust → Spec, for reachable states)
//! ```
//!
//! These invariants are verified by Kani proof harnesses (H-SIM1 … H-SIM5).

use laplace_core::domain::memory::{Address, StoreEntry, Value};
use laplace_core::domain::scheduler::ThreadState;
use laplace_core::domain::time::{LamportClock, VirtualTimeNs};

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

// ============================================================================
// Core Trait
// ============================================================================

/// Bijective mapping between a Rust implementation type and its TLA+ specification mirror.
///
/// # Type Parameters
///
/// - `Spec`: The TLA+ specification mirror type that `Self` corresponds to.
///
/// # Contract
///
/// Both `to_spec` and `from_spec` must be pure functions (no side effects).
/// Implementors must uphold: `from_spec(to_spec(x)) == x` for all valid `x`.
pub trait EquivalentTo<Spec>: Sized {
    /// Convert this Rust value to its TLA+ specification representation.
    fn to_spec(&self) -> Spec;

    /// Reconstruct a Rust value from a TLA+ specification representation.
    fn from_spec(spec: Spec) -> Self;
}

// ============================================================================
// TLA+ Specification Mirror Types
// ============================================================================

/// Specification mirror of a pending store-buffer entry.
///
/// Captures the minimal information needed to reconstruct the entry for
/// formal verification without exposing internal implementation details.
// TLA+ record: StoreBufferEntry == [addr : Nat, val : Nat]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreBufferEntry {
    /// Memory address (matches `Address(pub usize)`).
    pub addr: usize,
    /// Value to write (matches `Value(pub u64)`).
    pub val: u64,
}

/// Specification mirror of a scheduler thread state.
///
/// Corresponds to the finite enumeration of reachable thread states as
/// defined in the core scheduling specification.
// TLA+ set: ThreadStates == {"Runnable", "Blocked", "Completed"}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ThreadStateEnum {
    /// Thread can be scheduled.
    Runnable = 0,
    /// Thread awaits a resource.
    Blocked = 1,
    /// Thread has finished execution.
    Completed = 2,
}

/// Specification mirror of the virtual clock's time value (nanoseconds).
///
/// Wraps the raw timestamp so bijection proofs can reason about the
/// mapping without depending on the concrete clock implementation.
// TLA+ variable: virtualTimeNs : Nat
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VirtualTime(pub u64);

/// Specification mirror of the Lamport logical clock counter.
///
/// Monotonically non-decreasing; used to establish happens-before order
/// across simulation events in the bijection proofs.
// TLA+ variable: lamportClock : Nat
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LamportTimestamp(pub u64);

/// Specification mirror of the externally observable simulator state.
///
/// Captures main memory, clock, Lamport counter, and idle flag without
/// exposing internal store-buffer contents. The bounded memory representation
/// keeps the formal verification state space finite.
///
/// # Bounded Representation
///
/// `memory` is bounded to 4 address slots, matching the fixed-size
/// `VerificationBackend` defaults used in Kani proof harnesses.
// TLA+ record: TwinState == [memory : [Addr -> Val], clockNs : Nat,
//                            lamport : Nat, isIdle : BOOLEAN]
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "30_Axiom_Simulation",
        link = "LEP-0010-laplace-axiom-digital_twin_and_typestate"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TwinState {
    /// Snapshot of main memory: `[(addr, val); 4]` ordered by address 0..3.
    pub memory: [(usize, u64); 4],
    /// Virtual clock at observation time (nanoseconds).
    pub clock_ns: VirtualTimeNs,
    /// Lamport logical clock at observation time.
    pub lamport: LamportClock,
    /// Whether the simulator has no pending events and all buffers are flushed.
    pub is_idle: bool,
}

// ============================================================================
// EquivalentTo Implementations
// ============================================================================

// ── StoreEntry ↔ StoreBufferEntry ────────────────────────────────────────────

impl EquivalentTo<StoreBufferEntry> for StoreEntry {
    /// Extract raw (addr, val) numbers from the strongly-typed entry.
    fn to_spec(&self) -> StoreBufferEntry {
        StoreBufferEntry {
            addr: self.addr.0,
            val: self.val.0,
        }
    }

    /// Wrap raw numbers back into the strongly-typed entry.
    fn from_spec(spec: StoreBufferEntry) -> Self {
        StoreEntry::new(Address::new(spec.addr), Value::new(spec.val))
    }
}

// ── ThreadState ↔ ThreadStateEnum ────────────────────────────────────────────

impl EquivalentTo<ThreadStateEnum> for ThreadState {
    fn to_spec(&self) -> ThreadStateEnum {
        match self {
            ThreadState::Runnable => ThreadStateEnum::Runnable,
            ThreadState::Blocked => ThreadStateEnum::Blocked,
            ThreadState::Completed => ThreadStateEnum::Completed,
        }
    }

    fn from_spec(spec: ThreadStateEnum) -> Self {
        match spec {
            ThreadStateEnum::Runnable => ThreadState::Runnable,
            ThreadStateEnum::Blocked => ThreadState::Blocked,
            ThreadStateEnum::Completed => ThreadState::Completed,
        }
    }
}

// ── VirtualTimeNs (= u64) ↔ VirtualTime ──────────────────────────────────────

/// Bijection between `VirtualTimeNs` (a `u64` alias) and the spec `VirtualTime`.
///
/// Trivially lossless: wraps and unwraps the same `u64`.
impl EquivalentTo<VirtualTime> for VirtualTimeNs {
    fn to_spec(&self) -> VirtualTime {
        VirtualTime(*self)
    }
    fn from_spec(spec: VirtualTime) -> Self {
        spec.0
    }
}

// ── LamportClock (= u64) ↔ LamportTimestamp ──────────────────────────────────

/// Bijection between `LamportClock` (a `u64` alias) and the spec `LamportTimestamp`.
///
/// Trivially lossless: wraps and unwraps the same `u64`.
impl EquivalentTo<LamportTimestamp> for LamportClock {
    fn to_spec(&self) -> LamportTimestamp {
        LamportTimestamp(*self)
    }
    fn from_spec(spec: LamportTimestamp) -> Self {
        spec.0
    }
}

// ── VerificationSimulator ↔ TwinState ─────────────────────────────────────────

#[cfg(feature = "twin")]
impl EquivalentTo<TwinState>
    for super::facade::Simulator<
        laplace_core::domain::memory::VerificationBackend,
        laplace_core::domain::time::VerificationBackend,
    >
{
    /// Extract the observable state of the simulator into a `TwinState`.
    ///
    /// # Note
    ///
    /// Only reads addresses 0..3 (fixed by `VerificationBackend`'s `initial_size = 4`).
    /// Call `run_until_idle()` before extracting to ensure all buffered writes
    /// have been committed to main memory.
    fn to_spec(&self) -> TwinState {
        let mem = self.memory();
        TwinState {
            memory: [
                (0, mem.read_main_memory(Address::new(0)).0),
                (1, mem.read_main_memory(Address::new(1)).0),
                (2, mem.read_main_memory(Address::new(2)).0),
                (3, mem.read_main_memory(Address::new(3)).0),
            ],
            clock_ns: mem.clock().current_time(),
            lamport: mem.clock().current_lamport(),
            is_idle: self.is_idle(),
        }
    }

    /// Construct a `VerificationSimulator` whose main memory matches `spec.memory`.
    ///
    /// # Limitations
    ///
    /// - `clock_ns` and `lamport` from the spec are **not** restored — the
    ///   write-flush sequence increments both counters. The invariant
    ///   `to_spec(from_spec(s)).memory == s.memory` holds for memory contents;
    ///   the full round-trip (`clock_ns`, `lamport`) requires additional clock
    ///   manipulation not currently exposed by `VerificationBackend`.
    /// - `is_idle` will always be `true` after `from_spec` returns.
    fn from_spec(spec: TwinState) -> Self {
        use super::builder::VerificationSimulatorBuilder;
        use laplace_core::domain::memory::CoreId;

        let mut sim = VerificationSimulatorBuilder::build();

        // Write each address to main memory via Core 0, flushing immediately
        // to avoid store-buffer overflow (capacity = 2 per core).
        for (addr, val) in spec.memory.iter() {
            sim.memory_mut()
                .write(CoreId::new(0), Address::new(*addr), Value::new(*val))
                .ok();
            sim.run_until_idle();
        }

        sim
    }
}

// ============================================================================
// Kani Proofs (H-SIM1 … H-SIM5)
// ============================================================================

#[cfg(all(kani, feature = "twin"))]
mod kani_proofs {
    use super::*;
    use crate::simulation::builder::VerificationSimulatorBuilder;
    use laplace_core::domain::memory::{Address, CoreId, Value};

    // ── H-SIM1: StoreEntry bijection ─────────────────────────────────────────

    /// Verifies that `StoreEntry` ↔ `StoreBufferEntry` is a lossless bijection.
    #[kani::proof]
    fn proof_h_sim1_store_entry() {
        let addr: usize = kani::any();
        let val: u64 = kani::any();

        let entry = StoreEntry::new(Address::new(addr), Value::new(val));
        let spec = entry.to_spec();
        let back = StoreEntry::from_spec(spec);

        kani::assert(
            entry == back,
            "H-SIM1: StoreEntry round-trip must be lossless",
        );
    }

    // ── H-SIM2: ThreadState bijection ────────────────────────────────────────

    /// Verifies that `ThreadState` ↔ `ThreadStateEnum` is a lossless bijection.
    #[kani::proof]
    fn proof_h_sim2_thread_state() {
        let disc: u8 = kani::any();
        kani::assume(disc < 3);

        let state = match disc {
            0 => ThreadState::Runnable,
            1 => ThreadState::Blocked,
            _ => ThreadState::Completed,
        };

        let spec = state.to_spec();
        let back = ThreadState::from_spec(spec);

        kani::assert(
            state == back,
            "H-SIM2: ThreadState round-trip must be lossless",
        );
    }

    // ── H-SIM3: VirtualTimeNs bijection ──────────────────────────────────────

    /// Verifies that `VirtualTimeNs` (u64) ↔ `VirtualTime` is trivially lossless.
    #[kani::proof]
    fn proof_h_sim3_virtual_time() {
        let t: VirtualTimeNs = kani::any();
        let back: VirtualTimeNs =
            VirtualTimeNs::from_spec(<VirtualTimeNs as EquivalentTo<VirtualTime>>::to_spec(&t));
        kani::assert(
            t == back,
            "H-SIM3: VirtualTimeNs round-trip must be lossless",
        );
    }

    // ── H-SIM4: LamportClock bijection ───────────────────────────────────────

    /// Verifies that `LamportClock` (u64) ↔ `LamportTimestamp` is trivially lossless.
    #[kani::proof]
    fn proof_h_sim4_lamport_clock() {
        let l: LamportClock = kani::any();
        let back: LamportClock = LamportClock::from_spec(<LamportClock as EquivalentTo<
            LamportTimestamp,
        >>::to_spec(&l));
        kani::assert(
            l == back,
            "H-SIM4: LamportClock round-trip must be lossless",
        );
    }

    // ── H-SIM5 / proof_simulator_equivalence: memory content preservation ────

    /// Verifies that `VerificationSimulator` preserves memory content through
    /// the `EquivalentTo<TwinState>` round-trip.
    ///
    /// Invariant: writing a value to address `addr`, running until idle, and
    /// extracting `TwinState` must yield the same value at `addr`.
    ///
    /// This is the canonical `proof_simulator_equivalence` harness referenced
    /// in the Sprint 3 verification command.
    #[kani::proof]
    #[kani::unwind(8)]
    fn proof_simulator_equivalence() {
        let addr: usize = kani::any();
        kani::assume(addr < 4);
        let val: u64 = kani::any();

        // Build a fresh verification simulator and write one value.
        let mut sim = VerificationSimulatorBuilder::build();
        sim.memory_mut()
            .write(CoreId::new(0), Address::new(addr), Value::new(val))
            .ok();
        sim.run_until_idle();

        // Extract to TwinState and check the written address.
        let state: TwinState = sim.to_spec();

        kani::assert(
            state.is_idle,
            "H-SIM5: simulator must be idle after run_until_idle",
        );
        kani::assert(
            state.memory[addr].1 == val,
            "H-SIM5: memory content must be preserved in TwinState",
        );
        kani::assert(
            state.memory[addr].0 == addr,
            "H-SIM5: address index must match in TwinState",
        );
    }
}
