//! Entropy seed primitives for deterministic simulation
//!
//! Provides generalized entropy seed types shared across the platform:
//! - [`ContextId`]: A generic context identifier (generalizes domain-specific IDs like VUID)
//! - [`LocalSeed`]: Deterministically derived seed for a context
//! - [`SeedAssignment`]: An auditable record of a seed assignment event
//! - [`GlobalSeedConfig`]: Root configuration for seed derivation

use std::fmt;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ContextId
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Generic Context Identifier (u64-based).
///
/// Generalizes the concept of a domain-specific identifier (e.g., a Virtual User ID)
/// for use across any context-scoped entity.
///
/// # TLA+ Correspondence
/// In Kraken, corresponds to `vu_id` in KrakenDNA.tla. Values range from 1 to MaxContexts.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Entropy",
        link = "LEP-0015-laplace-interfaces-deterministic_entropy"
    )
)]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct ContextId(pub u64);

impl ContextId {
    /// Create a new ContextId from a raw u64.
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    /// Get the inner u64 value.
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for ContextId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Ctx#{}", self.0)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// LocalSeed
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Sovereign Local Seed derived from a global seed and a [`ContextId`].
///
/// Given the same global seed and `ContextId`, the derived `LocalSeed` is
/// always identical across all platforms and runs, ensuring reproducibility.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Entropy",
        link = "LEP-0015-laplace-interfaces-deterministic_entropy"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalSeed(u64);

impl LocalSeed {
    /// Create a `LocalSeed` from a raw u64 value.
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// Get the inner u64 seed value.
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for LocalSeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Seed(0x{:016x})", self.0)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// SeedAssignment
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Auditable record of a deterministic seed assignment event.
///
/// # TLA+ Correspondence
/// Corresponds to `SeedAssignmentEvent` in KrakenDNA.tla.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Entropy",
        link = "LEP-0015-laplace-interfaces-deterministic_entropy"
    )
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedAssignment {
    /// The context being assigned a seed.
    pub ctx_id: ContextId,

    /// The deterministically derived local seed.
    pub local_seed: LocalSeed,

    /// Lamport clock timestamp at assignment time.
    pub lamport_ts: u64,
}

impl SeedAssignment {
    /// Create a new seed assignment record.
    pub fn new(ctx_id: ContextId, local_seed: LocalSeed, lamport_ts: u64) -> Self {
        Self {
            ctx_id,
            local_seed,
            lamport_ts,
        }
    }
}

impl fmt::Display for SeedAssignment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SeedAssignment {{ {}, {}, lamport: {} }}",
            self.ctx_id, self.local_seed, self.lamport_ts
        )
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GlobalSeedConfig
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Root configuration for seed derivation in a simulation run.
///
/// # TLA+ Correspondence
/// Corresponds to the `GlobalSeed` constant in KrakenDNA.tla.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Entropy",
        link = "LEP-0015-laplace-interfaces-deterministic_entropy"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlobalSeedConfig {
    /// The global seed value (typically from scenario config or user input).
    pub seed: u64,

    /// Lamport clock modulus for state space bounding.
    pub lamport_mod: u64,

    /// Maximum number of contexts (quota enforcement).
    pub max_contexts: usize,
}

impl GlobalSeedConfig {
    /// Create a new global seed configuration.
    pub fn new(seed: u64, lamport_mod: u64, max_contexts: usize) -> Self {
        Self {
            seed,
            lamport_mod,
            max_contexts,
        }
    }

    /// Create a default configuration for testing.
    pub fn test_config() -> Self {
        Self {
            seed: 12345,
            lamport_mod: 8,
            max_contexts: 3,
        }
    }
}
