// SPDX-License-Identifier: Apache-2.0
#![deny(clippy::all)]
#![allow(unexpected_cfgs)]

//! Public Laplace harness DSL and scenario metadata.
//!
//! Private executable providers live in `laplace-cloud`.

pub mod dsl;

#[cfg(feature = "registry")]
pub mod registry;

#[cfg(laplace_private_harness)]
pub mod scenarios;

#[cfg(laplace_private_harness)]
pub mod external_audit;
