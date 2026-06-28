// SPDX-License-Identifier: Apache-2.0
#![deny(clippy::all)]
#![allow(unexpected_cfgs)]

//! Public Laplace harness DSL and scenario metadata.
//!
//! Private executable providers live in `laplace-cloud`.

pub mod dsl;

#[cfg(feature = "registry")]
pub mod registry;

// Built-in verification scenarios + external-crate audits. Gated behind the
// `scenarios` feature (which pulls in `registry`) so the private cloud build can
// link them into the CLI's harness registry, while the thin public default stays
// DSL-only. The legacy `laplace_private_harness` cfg is still honored for
// out-of-tree private builds that set it via RUSTFLAGS.
#[cfg(any(feature = "scenarios", laplace_private_harness))]
pub mod scenarios;

#[cfg(any(feature = "scenarios", laplace_private_harness))]
pub mod external_audit;
