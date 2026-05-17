#![deny(clippy::all, clippy::pedantic)]

//! Laplace Harness Registry — centralised verification scenarios for Axiom Oracle.
//!
//! Requires `feature = "twin"` and `feature = "verification"` to be active.

#[cfg(all(feature = "twin", feature = "verification"))]
pub mod registry;

#[cfg(all(feature = "twin", feature = "verification"))]
pub mod scenarios;

#[cfg(all(feature = "twin", feature = "verification", feature = "internal-audit"))]
pub mod external_audit;
