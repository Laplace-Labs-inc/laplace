// SPDX-License-Identifier: Apache-2.0
//! Laplace DPOR — public reference partial-order reduction algorithms
//!
//! This crate provides the core concurrency verification algorithms:
//! - Classic DPOR with vector-clock causality tracking
//! - Kani formal proofs (cfg kani)

pub mod dpor;
pub mod error;

pub use dpor::{DporScheduler, DporStats, Operation, StepRecord, TinyBitSet, VectorClock};
pub use error::LaplaceError;
