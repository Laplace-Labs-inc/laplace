// SPDX-License-Identifier: Apache-2.0
//! Laplace Ki-DPOR — Deterministic Partial-Order Reduction Algorithms
//!
//! This crate provides the core concurrency verification algorithms:
//! - Classic DPOR with vector-clock causality tracking
//! - Ki-DPOR (A*-guided intelligent DPOR)
//! - Kani formal proofs (cfg kani)

pub mod dpor;

pub use dpor::{
    DporScheduler, DporStats, Operation, StepRecord, TinyBitSet, VectorClock,
};
