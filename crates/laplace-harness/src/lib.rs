// SPDX-License-Identifier: Apache-2.0
#![deny(clippy::all)]

//! Laplace Harness Registry - centralised verification scenarios for Axiom Oracle.
//!
//! Requires `feature = "twin"` and `feature = "verification"` to be active.

pub mod dsl;

#[cfg(all(feature = "twin", feature = "verification"))]
pub mod registry;

#[cfg(all(feature = "twin", feature = "verification"))]
pub mod scenarios;

#[cfg(all(feature = "twin", feature = "verification", feature = "internal-audit"))]
pub mod external_audit;
