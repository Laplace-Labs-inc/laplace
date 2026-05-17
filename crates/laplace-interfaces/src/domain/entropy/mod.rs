//! Entropy domain contracts — seed primitives and trait definitions

pub mod traits;
pub mod types;

pub use traits::Entropy;
pub use types::{ContextId, GlobalSeedConfig, LocalSeed, SeedAssignment};
