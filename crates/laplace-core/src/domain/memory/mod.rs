//! Memory Model Abstraction Layer
//!
//! Provides a unified memory abstraction that supports both production and verification
//! workloads. The trait-based design enables zero-cost compile-time polymorphism through
//! generic specialization, allowing different backends to be swapped without runtime overhead.
//!
//! # Architectural Overview
//!
//! The memory layer consists of four components:
//!
//! **Types** (`types.rs`): Core data types that form the foundation of the memory model.
//! These types correspond directly to the TLA+ specification: addresses, values, core IDs,
//! store entries, and consistency models.
//!
//! **Traits** (`traits.rs`): The `MemoryBackend` trait defines the abstract interface for
//! memory operations. All implementations must satisfy this contract, enabling compile-time
//! polymorphism through generic specialization.
//!
//! **Backends**: Two complementary implementations provide the flexibility to optimize for
//! different scenarios. `ProductionBackend` uses concurrent data structures suitable for
//! real workloads, while `VerificationBackend` uses bounded fixed-size arrays optimized
//! for symbolic execution with Kani.
//!
//! **SimulatedMemory** (`simulated.rs`): The high-level orchestrator that combines a
//! memory backend with a virtual clock for event scheduling. This module provides the
//! primary API for read, write, fence, and flush operations with direct TLA+ correspondence.
//!
//! # Design Principles
//!
//! **Fractal Integrity**: Each module has a single, well-defined responsibility. Types
//! remain separate from traits, which remain separate from implementations.
//!
//! **Native-First**: Core logic is pure Rust with no infrastructure dependencies. The
//! abstraction layer handles composition without requiring framework support.
//!
//! **Deterministic Context**: All operations accept explicit parameters and maintain no
//! implicit global state beyond the backend instance itself.
//!
//! # Usage
//!
//! In production scenarios, use `ProductionBackend` for scalable concurrent workloads:
//!
//! ```ignore
//! use laplace_core::domain::memory::{ProductionBackend, MemoryBackend};
//!
//! let mut backend = ProductionBackend::new(4, 256);
//! backend.write_main(0x1000, 42);
//! let value = backend.read_main(0x1000); // Returns 42
//! ```
//!
//! For a complete memory system with event scheduling, use `SimulatedMemory`:
//!
//! ```ignore
//! use laplace_core::domain::memory::{ProductionBackend, MemoryConfig, SimulatedMemory};
//! use laplace_core::domain::time::VirtualClock;
//!
//! let backend = ProductionBackend::new(4, 256);
//! let clock = VirtualClock::new();
//! let mut memory = SimulatedMemory::new(backend, clock, MemoryConfig::default());
//!
//! memory.write(0, 0x1000, 42)?;
//! let value = memory.read(0, 0x1000);
//! ```
//!
//! In verification scenarios, use `VerificationBackend` with Axiom for formal verification:
//!
//! ```ignore
//! #[cfg(feature = "twin")]
//! use laplace_core::domain::memory::{VerificationBackend, MemoryBackend};
//!
//! let mut backend = VerificationBackend::new();
//! // Kani can fully explore the state space of this backend
//! ```
//!
//! # TLA+ Correspondence
//!
//! The memory layer directly corresponds to the SimulatedMemory.tla specification:
//!
//! ```tla
//! VARIABLES mainMemory, storeBuffers
//! ```
//!
//! - `mainMemory[addr] → val`: Direct memory access via `read_main()` / `write_main()`
//! - `storeBuffers[core]`: Per-core FIFO queues accessed via `buffer_push()` / `buffer_pop()`
//! - `BufferLookup(core, addr)`: Load forwarding via `buffer_lookup()`

#[cfg(feature = "verification")]
pub mod production;
pub mod simulated;
pub mod traits;
pub mod types;

#[cfg(any(test, feature = "twin", kani))]
pub mod verification;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Public Re-exports: Unified Memory API
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// Types - Always available
pub use types::{Address, ConsistencyModel, CoreId, MemoryConfig, MemoryOp, StoreEntry, Value};

// Traits - Always available
pub use traits::{ConfigurableBackend, MemoryBackend};

// SimulatedMemory - Always available
pub use simulated::SimulatedMemory;

// Production backend - requires dashmap + parking_lot (verification feature)
#[cfg(feature = "verification")]
pub use production::ProductionBackend;

// Verification backend - Feature-gated for test/Axiom/Kani scenarios
#[cfg(any(test, feature = "twin", kani))]
pub use verification::VerificationBackend;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Formal Verification Harnesses
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(all(kani, feature = "twin"))]
mod proofs;
