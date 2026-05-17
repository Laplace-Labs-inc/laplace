//! Memory domain contracts.
//!
//! Provides the canonical type and trait definitions for the Laplace memory abstraction.
//! Concrete backend implementations live in `laplace-core`; this module only defines
//! the shared interface consumed by both the core and the twin crates.
//!
//! # Contents
//!
//! - [`types`]: `Address`, `Value`, `CoreId`, `StoreEntry`, `MemoryOp`, `ConsistencyModel`, `MemoryConfig`
//! - [`traits`]: `MemoryBackend`, `ConfigurableBackend`

pub mod traits;
pub mod types;

pub use traits::{ConfigurableBackend, MemoryBackend};
pub use types::{Address, ConsistencyModel, CoreId, MemoryConfig, MemoryOp, StoreEntry, Value};
