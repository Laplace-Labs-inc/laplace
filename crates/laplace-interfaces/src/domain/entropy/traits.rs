//! Entropy trait contract

use std::fmt;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Canonical trait for entropy sources in the Laplace platform.
///
/// Implementations must be `Send + Sync` to enable global registration and
/// thread-safe access from any context.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Entropy",
        link = "LEP-0015-laplace-interfaces-deterministic_entropy"
    )
)]
pub trait Entropy: Send + Sync + fmt::Debug {
    /// Generate a random u64 value.
    fn next_u64(&self) -> u64;

    /// Fill a buffer with random bytes.
    fn fill_bytes(&self, dest: &mut [u8]);

    /// Generate a random value uniformly distributed in `[0, max)`.
    ///
    /// Must avoid modulo bias by rejecting values in the "danger zone".
    fn next_range(&self, max: u64) -> u64;
}
