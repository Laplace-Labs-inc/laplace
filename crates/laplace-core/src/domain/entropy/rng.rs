// SPDX-License-Identifier: Apache-2.0
//! Deterministic PRNG for Laplace Core Domain
//!
//! Provides a ChaCha8-based deterministic random number generator with
//! snapshot/restore capability for DPOR time-machine rollback.
//!
//! # Feature Gate
//! Only compiled when the `twin` feature is active to prevent test-only
//! randomness from appearing in production binaries.

#![cfg(feature = "twin")]

use std::fmt;

use rand_chacha::rand_core::{RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;

use super::seed::{ContextId, LocalSeed};

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// RngSnapshot
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Opaque snapshot of [`DeterministicRng`] internal state.
///
/// Used by DPOR time-machine rollback to restore a previous RNG state
/// exactly, enabling deterministic replay of any execution branch from
/// a checkpoint.
///
/// # Usage
/// ```ignore
/// let snapshot = rng.capture_snapshot();
/// // ... consume entropy ...
/// rng.restore_snapshot(snapshot); // back to checkpoint
/// ```
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "20_Core_Entropy",
        link = "LEP-0019-laplace-kraken-cryptographic_rng_and_zero_alloc_snapshot"
    )
)]
#[derive(Clone)]
pub struct RngSnapshot {
    rng: ChaCha8Rng,
}

impl fmt::Debug for RngSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RngSnapshot").finish_non_exhaustive()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DeterministicRng
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Deterministic PRNG bound to a [`ContextId`] and its [`LocalSeed`].
///
/// Wraps `ChaCha8Rng` with context identity metadata. Given the same
/// `ContextId` and `LocalSeed`, produces identical sequences on any platform.
///
/// # Guarantees
/// - Same context + seed → same sequence (deterministic across machines)
/// - Unbiased range generation (rejection sampling, no modulo bias)
/// - Cloneable: safe to fork for parallel exploration
///
/// # DPOR Support
/// Provides [`capture_snapshot`] and [`restore_snapshot`] for DPOR
/// time-machine rollback, enabling exploration of alternative execution paths
/// without re-running the entire simulation from scratch.
///
/// [`capture_snapshot`]: DeterministicRng::capture_snapshot
/// [`restore_snapshot`]: DeterministicRng::restore_snapshot
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "20_Core_Entropy",
        link = "LEP-0019-laplace-kraken-cryptographic_rng_and_zero_alloc_snapshot"
    )
)]
#[derive(Debug, Clone)]
pub struct DeterministicRng {
    /// Context identifier for this RNG instance.
    ctx_id: ContextId,

    /// Seed used to initialize this RNG.
    seed: LocalSeed,

    /// Underlying ChaCha8 pseudo-random number generator.
    rng: ChaCha8Rng,
}

impl DeterministicRng {
    /// Create a new deterministic RNG for a context.
    ///
    /// The RNG state is fully determined by `ctx_id` and `seed`;
    /// two instances with the same arguments produce identical sequences.
    pub fn new(ctx_id: ContextId, seed: LocalSeed) -> Self {
        let rng = ChaCha8Rng::from_seed(Self::seed_to_array(seed));
        Self { ctx_id, seed, rng }
    }

    /// Expand a [`LocalSeed`] into the 32-byte array required by ChaCha8.
    ///
    /// The 8-byte seed material is repeated 4 times to fill the 32-byte array,
    /// preserving full determinism while satisfying the ChaCha8 seed size.
    fn seed_to_array(seed: LocalSeed) -> [u8; 32] {
        let seed_u64 = seed.as_u64();
        let mut array = [0u8; 32];
        for i in 0..4 {
            let bytes = seed_u64.to_le_bytes();
            array[i * 8..(i + 1) * 8].copy_from_slice(&bytes);
        }
        array
    }

    // ── Accessors ──────────────────────────────────────────────────────────────

    /// Get the context ID associated with this RNG.
    pub fn ctx_id(&self) -> ContextId {
        self.ctx_id
    }

    /// Get the seed associated with this RNG.
    pub fn seed(&self) -> LocalSeed {
        self.seed
    }

    // ── Generation ─────────────────────────────────────────────────────────────

    /// Generate a uniformly random `u64` in `[0, u64::MAX]`.
    pub fn next_u64(&mut self) -> u64 {
        self.rng.next_u64()
    }

    /// Generate a uniformly random `u32` in `[0, u32::MAX]`.
    pub fn next_u32(&mut self) -> u32 {
        self.rng.next_u32()
    }

    /// Generate a uniformly random value in `[0, max)` with no modulo bias.
    ///
    /// Uses rejection sampling ("Apple Logic") to guarantee uniform distribution:
    /// values in the "danger zone" where `u64::MAX % max != 0` are discarded.
    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(
            layer = "20_Core_Entropy",
            link = "LEP-0003-laplace-core-entropy_determinism"
        )
    )]
    pub fn next_range(&mut self, max: u64) -> u64 {
        if max <= 1 {
            return 0;
        }
        let zone = u64::MAX - (u64::MAX % max);
        loop {
            let v = self.rng.next_u64();
            if v < zone {
                return v % max;
            }
        }
    }

    /// Generate a random boolean value.
    pub fn next_bool(&mut self) -> bool {
        self.rng.next_u32() & 1 == 1
    }

    /// Generate a value in `[min, max)`.
    ///
    /// Returns `min` if `min >= max`.
    pub fn next_range_inclusive(&mut self, min: u64, max: u64) -> u64 {
        if min >= max {
            return min;
        }
        min + self.next_range(max - min)
    }

    /// Fill a buffer with uniformly distributed random bytes.
    pub fn fill_bytes(&mut self, buf: &mut [u8]) {
        self.rng.fill_bytes(buf);
    }

    // ── State Management ───────────────────────────────────────────────────────

    /// Reset the RNG to its initial state.
    ///
    /// After reset, the next generated value is identical to the value
    /// from a freshly created RNG with the same seed.
    pub fn reset(&mut self) {
        self.rng = ChaCha8Rng::from_seed(Self::seed_to_array(self.seed));
    }

    /// Capture the current RNG state for DPOR time-machine rollback.
    ///
    /// The returned [`RngSnapshot`] can be passed to [`restore_snapshot`] to
    /// return the RNG to exactly this point, enabling deterministic replay of
    /// any alternative execution branch.
    ///
    /// [`restore_snapshot`]: DeterministicRng::restore_snapshot
    pub fn capture_snapshot(&self) -> RngSnapshot {
        RngSnapshot {
            rng: self.rng.clone(),
        }
    }

    /// Restore the RNG state from a previously captured snapshot.
    ///
    /// After restoration the RNG produces the same sequence as it would
    /// have produced from the moment of capture, enabling DPOR rollback
    /// to explore alternative interleavings.
    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(
            layer = "20_Core_Entropy",
            link = "LEP-0003-laplace-core-entropy_determinism"
        )
    )]
    pub fn restore_snapshot(&mut self, snapshot: RngSnapshot) {
        self.rng = snapshot.rng;
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entropy::SeedDerive;

    fn make_rng(ctx: u64, global: u64) -> DeterministicRng {
        let ctx_id = ContextId::new(ctx);
        let seed = LocalSeed::derive(global, ctx_id);
        DeterministicRng::new(ctx_id, seed)
    }

    #[test]
    fn test_rng_creation() {
        let rng = make_rng(42, 12345);
        assert_eq!(rng.ctx_id(), ContextId::new(42));
        assert_eq!(rng.seed(), LocalSeed::derive(12345, ContextId::new(42)));
    }

    #[test]
    fn test_rng_determinism() {
        let mut rng1 = make_rng(42, 12345);
        let mut rng2 = make_rng(42, 12345);

        for _ in 0..100 {
            assert_eq!(rng1.next_u64(), rng2.next_u64());
        }
    }

    #[test]
    fn test_rng_different_seeds_differ() {
        let mut rng1 = make_rng(1, 12345);
        let mut rng2 = make_rng(2, 12345);

        let different = (0..10).any(|_| rng1.next_u64() != rng2.next_u64());
        assert!(different);
    }

    #[test]
    fn test_rng_next_range_in_bounds() {
        let mut rng = make_rng(1, 12345);
        for _ in 0..100 {
            let val = rng.next_range(100);
            assert!(val < 100);
        }
    }

    #[test]
    fn test_rng_next_range_inclusive_in_bounds() {
        let mut rng = make_rng(1, 12345);
        for _ in 0..100 {
            let val = rng.next_range_inclusive(10, 20);
            assert!((10..20).contains(&val));
        }
    }

    #[test]
    fn test_rng_reset() {
        let mut rng = make_rng(1, 12345);
        let val1 = rng.next_u64();
        let val2 = rng.next_u64();

        rng.reset();

        assert_eq!(rng.next_u64(), val1);
        assert_eq!(rng.next_u64(), val2);
    }

    #[test]
    fn test_rng_snapshot_capture_restore() {
        let mut rng = make_rng(1, 12345);

        // Consume some entropy
        let _ = rng.next_u64();
        let _ = rng.next_u64();

        // Capture state and record the next value
        let snapshot = rng.capture_snapshot();
        let checkpoint_val = rng.next_u64();

        // Consume more entropy (changing state)
        let _ = rng.next_u64();
        let _ = rng.next_u64();

        // Restore and verify we get the same value again
        rng.restore_snapshot(snapshot);
        assert_eq!(rng.next_u64(), checkpoint_val);
    }

    #[test]
    fn test_rng_snapshot_clone_independence() {
        let mut rng = make_rng(1, 12345);
        let snapshot = rng.capture_snapshot();

        // Advance original
        let val_after = rng.next_u64();

        // Restore and verify
        rng.restore_snapshot(snapshot);
        assert_eq!(rng.next_u64(), val_after);
    }

    #[test]
    fn test_rng_next_bool_roughly_uniform() {
        let mut rng = make_rng(1, 12345);
        let true_count: u32 = (0..100).filter(|_| rng.next_bool()).count() as u32;
        assert!(true_count > 20 && true_count < 80);
    }
}
