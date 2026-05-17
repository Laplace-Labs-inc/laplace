//! Internal Seed Manager — proprietary derivation formula
//!
//! This module encapsulates the seed derivation formula that must NOT be
//! exposed via `laplace-interfaces`. All callers within `laplace-core` and
//! `laplace-kraken` must use [`derive_local_seed`] (or the [`SeedDerive`] /
//! [`SeedVerify`] extension traits) instead of any removed public API on
//! [`LocalSeed`] or [`SeedAssignment`].

use laplace_interfaces::domain::entropy::types::{ContextId, LocalSeed, SeedAssignment};

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Core derivation function (not part of laplace-interfaces public API)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Derive a `LocalSeed` from a global seed and a context ID.
///
/// This is the authoritative, internal implementation of the seed derivation
/// formula. It is intentionally kept out of `laplace-interfaces` to prevent
/// IP leakage in open-source distributions.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "20_Core_Entropy",
        link = "LEP-0003-laplace-core-entropy_determinism"
    )
)]
pub fn derive_local_seed(global_seed: u64, ctx_id: ContextId) -> LocalSeed {
    let derived = global_seed.wrapping_add(ctx_id.as_u64().wrapping_mul(37));
    LocalSeed::new(derived)
}

/// Verify that a `SeedAssignment` is deterministically correct.
///
/// Returns `true` if the stored `local_seed` matches the value that
/// `derive_local_seed` would produce for `global_seed + assignment.ctx_id`.
pub fn verify_seed_assignment(assignment: &SeedAssignment, global_seed: u64) -> bool {
    derive_local_seed(global_seed, assignment.ctx_id) == assignment.local_seed
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Extension Traits (drop-in replacements for the removed interface methods)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Extension trait that restores `LocalSeed::derive(global, ctx)` ergonomics
/// without exposing the formula in `laplace-interfaces`.
///
/// Import this trait wherever `LocalSeed::derive(...)` or
/// `KrakenSeed::derive(...)` is used:
///
/// ```ignore
/// use laplace_core::domain::entropy::SeedDerive;
/// let seed = LocalSeed::derive(global_seed, ctx_id);
/// ```
pub trait SeedDerive: Sized {
    /// Derive a seed value from a global seed and a context ID.
    fn derive(global_seed: u64, ctx_id: ContextId) -> Self;
}

impl SeedDerive for LocalSeed {
    fn derive(global_seed: u64, ctx_id: ContextId) -> Self {
        derive_local_seed(global_seed, ctx_id)
    }
}

/// Extension trait that restores `SeedAssignment::verify(global)` ergonomics
/// without exposing the derivation formula in `laplace-interfaces`.
///
/// Import this trait wherever `assignment.verify(global_seed)` is used:
///
/// ```ignore
/// use laplace_core::domain::entropy::SeedVerify;
/// assert!(assignment.verify(global_seed));
/// ```
pub trait SeedVerify {
    /// Verify that this assignment was produced by the canonical derivation formula.
    fn verify(&self, global_seed: u64) -> bool;
}

impl SeedVerify for SeedAssignment {
    fn verify(&self, global_seed: u64) -> bool {
        verify_seed_assignment(self, global_seed)
    }
}
