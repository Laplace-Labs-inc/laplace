// SPDX-License-Identifier: Apache-2.0
//! Entropy Abstraction Layer
//!
//! Provides a unified interface for random number generation that supports both
//! production randomness (via `SystemEntropy`) and deterministic reproducible
//! sequences (via `DeterministicEntropy` for Axiom verification).
//!
//! # Sub-modules
//!
//! - [`seed`]: Generalized seed primitives — [`ContextId`], [`LocalSeed`],
//!   [`SeedAssignment`], [`GlobalSeedConfig`]
//! - [`rng`]: Deterministic PRNG — [`DeterministicRng`] with DPOR snapshot support
//!   (compiled only with `feature = "twin"`)
//!
//! # Architectural Role
//!
//! The entropy layer serves as the canonical source of non-determinism in the
//! Laplace platform. By abstracting entropy, we achieve:
//!
//! 1. **Verifiability**: Axiom environment can inject deterministic entropy to
//!    produce repeatable execution traces for formal verification.
//!
//! 2. **Production Safety**: SystemEntropy uses cryptographically-secure sources
//!    with no bias or predictability concerns.
//!
//! 3. **Zero Overhead**: SystemEntropy is zero-sized and delegates to standard
//!    Rust RNG sources with minimal indirection.

pub mod seed;
pub mod seed_manager;

#[cfg(feature = "twin")]
pub mod rng;

#[cfg(kani)]
mod proofs;

// Re-exports from seed sub-module (types now live in laplace-interfaces)
pub use seed::{ContextId, GlobalSeedConfig, LocalSeed, SeedAssignment};

// Internal derivation API — NOT in laplace-interfaces
pub use seed_manager::{derive_local_seed, verify_seed_assignment, SeedDerive, SeedVerify};

// Re-exports from rng sub-module (twin only)
#[cfg(feature = "twin")]
pub use rng::{DeterministicRng, RngSnapshot};

// Entropy trait re-exported from laplace-interfaces (authoritative source)
pub use laplace_interfaces::domain::entropy::Entropy;

#[cfg(feature = "twin")]
use std::cell::RefCell;
#[cfg(feature = "twin")]
use std::collections::HashMap;
#[cfg(feature = "twin")]
use std::fmt;
#[cfg(feature = "twin")]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "twin")]
use rand_chacha::rand_core::RngCore;

#[cfg(feature = "twin")]
use rand_chacha::rand_core::SeedableRng;

#[cfg(feature = "twin")]
use rand_chacha::ChaCha8Rng;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Production: SystemEntropy
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Production entropy source using the operating system's secure randomness.
///
/// `SystemEntropy` is a zero-sized marker struct that delegates to `rand::rngs::OsRng`
/// on each invocation. This ensures:
///
/// - **Cryptographic quality**: OsRng provides entropy from the OS kernel.
/// - **No state**: Each operation independently requests new randomness.
/// - **Zero allocation**: No heap allocation overhead.
///
/// # Thread Safety
///
/// `SystemEntropy` is `Send + Sync` and safe to share globally.
#[derive(Debug, Clone, Copy)]
pub struct SystemEntropy;

impl SystemEntropy {
    /// Create a new production entropy source.
    pub fn new() -> Self {
        Self
    }
}

impl Default for SystemEntropy {
    fn default() -> Self {
        Self::new()
    }
}

impl Entropy for SystemEntropy {
    fn next_u64(&self) -> u64 {
        rand::random::<u64>()
    }

    fn fill_bytes(&self, dest: &mut [u8]) {
        rand::fill(dest);
    }

    fn next_range(&self, max: u64) -> u64 {
        if max <= 1 {
            return 0;
        }
        rand::random_range(0..max)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Testing/Axiom: DeterministicEntropy
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Deterministic entropy source for reproducible testing and Axiom verification.
///
/// `DeterministicEntropy` wraps `ChaCha8Rng` with a `Mutex` to provide thread-safe,
/// deterministic random number generation. Given the same seed, it produces identical
/// sequences across machines and runs, enabling reproducible execution traces for
/// formal verification.
///
/// # Guarantees
///
/// - **Determinism**: Same seed → identical sequence.
/// - **Platform Independence**: Produces same bytes on any platform.
/// - **Cryptographic Quality**: ChaCha8 is suitable for simulation workloads.
/// - **Thread Safety**: Mutex protects state for global `Send + Sync` compliance.
///
/// # Feature Gating
///
/// Only compiled when tests are enabled or when the `twin` feature is active.
/// This prevents accidental use of test-only randomness in production binaries.
#[cfg(feature = "twin")]
#[derive(Clone)]
pub struct DeterministicEntropy {
    /// Seed value for initialization and reset tracking.
    seed: u64,

    /// Per-instance key into the thread-local PRNG map.
    instance_id: u64,
}

#[cfg(feature = "twin")]
static NEXT_DETERMINISTIC_ENTROPY_ID: AtomicU64 = AtomicU64::new(1);

#[cfg(feature = "twin")]
thread_local! {
    static THREAD_RNGS: RefCell<HashMap<u64, ChaCha8Rng>> = RefCell::new(HashMap::new());
}

#[cfg(feature = "twin")]
impl DeterministicEntropy {
    /// Create a new deterministic entropy source seeded with the given value.
    ///
    /// # Arguments
    ///
    /// - `seed`: A u64 value used to initialize the PRNG. The same seed always
    ///   produces the same sequence.
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            instance_id: NEXT_DETERMINISTIC_ENTROPY_ID.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// Expand a u64 seed into a [u8; 32] ChaCha8 seed array.
    fn seed_to_array(seed: u64) -> [u8; 32] {
        let mut array = [0u8; 32];
        let bytes = seed.to_le_bytes();

        for i in 0..4 {
            array[i * 8..(i + 1) * 8].copy_from_slice(&bytes);
        }

        array
    }

    /// Get the seed value used to initialize this entropy source.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Reset the PRNG to its initial state.
    ///
    /// After reset, the next generated value will be identical to the value
    /// from a freshly created entropy source with the same seed.
    pub fn reset(&self) {
        THREAD_RNGS.with(|rngs| {
            rngs.borrow_mut().remove(&self.instance_id);
        });
    }

    fn with_rng<R>(&self, f: impl FnOnce(&mut ChaCha8Rng) -> R) -> R {
        THREAD_RNGS.with(|rngs| {
            let mut rngs = rngs.borrow_mut();
            let rng = rngs
                .entry(self.instance_id)
                .or_insert_with(|| ChaCha8Rng::from_seed(Self::seed_to_array(self.seed)));
            f(rng)
        })
    }
}

#[cfg(feature = "twin")]
impl fmt::Debug for DeterministicEntropy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeterministicEntropy")
            .field("seed", &self.seed)
            .finish()
    }
}

#[cfg(feature = "twin")]
impl Entropy for DeterministicEntropy {
    fn next_u64(&self) -> u64 {
        self.with_rng(|rng| rng.next_u64())
    }

    fn fill_bytes(&self, dest: &mut [u8]) {
        self.with_rng(|rng| rng.fill_bytes(dest));
    }

    fn next_range(&self, max: u64) -> u64 {
        if max <= 1 {
            return 0;
        }

        self.with_rng(|rng| {
            let zone = u64::MAX - (u64::MAX % max);

            loop {
                let v = rng.next_u64();
                if v < zone {
                    return v % max;
                }
            }
        })
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_entropy_produces_values() {
        let entropy = SystemEntropy::new();
        let val1 = entropy.next_u64();
        let val2 = entropy.next_u64();

        assert!(val1 != 0 || val2 != 0);
        assert_ne!(val1, val2);
    }

    #[test]
    fn test_system_entropy_fill_bytes() {
        let entropy = SystemEntropy::new();
        let mut buf1 = [0u8; 32];
        let mut buf2 = [0u8; 32];

        entropy.fill_bytes(&mut buf1);
        entropy.fill_bytes(&mut buf2);

        assert_ne!(buf1, buf2);
    }

    #[test]
    fn test_system_entropy_next_range() {
        let entropy = SystemEntropy::new();

        for _ in 0..100 {
            let val = entropy.next_range(100);
            assert!(val < 100);
        }
    }

    #[cfg(feature = "twin")]
    #[test]
    fn test_deterministic_entropy_creation() {
        let entropy = DeterministicEntropy::new(0xDEADBEEF);
        assert_eq!(entropy.seed(), 0xDEADBEEF);
    }

    #[cfg(feature = "twin")]
    #[test]
    fn test_deterministic_entropy_determinism() {
        let entropy1 = DeterministicEntropy::new(12345);
        let entropy2 = DeterministicEntropy::new(12345);

        for _ in 0..100 {
            assert_eq!(entropy1.next_u64(), entropy2.next_u64());
        }
    }

    #[cfg(feature = "twin")]
    #[test]
    fn test_deterministic_entropy_different_seeds() {
        let entropy1 = DeterministicEntropy::new(111);
        let entropy2 = DeterministicEntropy::new(222);

        let mut different = false;
        for _ in 0..10 {
            if entropy1.next_u64() != entropy2.next_u64() {
                different = true;
                break;
            }
        }

        assert!(different);
    }

    #[cfg(feature = "twin")]
    #[test]
    fn test_deterministic_entropy_next_range() {
        let entropy = DeterministicEntropy::new(42);

        for _ in 0..100 {
            let val = entropy.next_range(100);
            assert!(val < 100);
        }
    }

    #[cfg(feature = "twin")]
    #[test]
    fn test_deterministic_entropy_no_modulo_bias() {
        let entropy = DeterministicEntropy::new(0xCAFEBABE);

        let mut counts = vec![0u32; 10];
        for _ in 0..10000 {
            let val = entropy.next_range(10);
            counts[val as usize] += 1;
        }

        for &count in &counts {
            assert!(count > 0, "Some values in range never appeared");
        }

        for (i, &count) in counts.iter().enumerate() {
            assert!(
                (800..=1200).contains(&count),
                "Slot {} had {} occurrences (expected ~1000)",
                i,
                count
            );
        }
    }

    #[cfg(feature = "twin")]
    #[test]
    fn test_entropy_thread_local_deterministic_with_same_seed() {
        let entropy1 = DeterministicEntropy::new(777);
        let entropy2 = DeterministicEntropy::new(777);

        let seq1: Vec<u64> = (0..8).map(|_| entropy1.next_u64()).collect();
        let seq2: Vec<u64> = (0..8).map(|_| entropy2.next_u64()).collect();

        assert_eq!(seq1, seq2);
    }

    #[cfg(feature = "twin")]
    #[test]
    fn test_entropy_different_threads_independent() {
        let entropy = DeterministicEntropy::new(888);
        let first_main = entropy.next_u64();
        let cloned = entropy.clone();
        let first_worker = std::thread::spawn(move || cloned.next_u64())
            .join()
            .unwrap();

        assert_eq!(first_main, first_worker);
        assert_ne!(entropy.next_u64(), first_worker);
    }

    #[cfg(feature = "twin")]
    #[test]
    fn test_deterministic_entropy_reset() {
        let entropy = DeterministicEntropy::new(0x1122);

        let val1 = entropy.next_u64();
        let val2 = entropy.next_u64();

        entropy.reset();

        assert_eq!(entropy.next_u64(), val1);
        assert_eq!(entropy.next_u64(), val2);
    }

    #[cfg(feature = "twin")]
    #[test]
    fn test_deterministic_entropy_fill_bytes() {
        let entropy = DeterministicEntropy::new(0x5678);

        let mut buf1 = [0u8; 64];
        entropy.fill_bytes(&mut buf1);

        entropy.reset();

        let mut buf2 = [0u8; 64];
        entropy.fill_bytes(&mut buf2);

        assert_eq!(buf1, buf2);
    }
}
