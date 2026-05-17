//! External audit zone — formal concurrency audits of third-party crates.
//!
//! Each sub-module models an external crate's concurrency behaviour using
//! Axiom's DPOR primitives, enabling exhaustive interleaving exploration
//! without a real async runtime.

pub mod deadpool;
