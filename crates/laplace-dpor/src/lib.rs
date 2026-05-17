//! Laplace Ki-DPOR — Deterministic Partial-Order Reduction Algorithms
//!
//! This crate provides the core concurrency verification algorithms:
//! - Classic DPOR with vector-clock causality tracking
//! - Ki-DPOR (A*-guided intelligent DPOR)
//! - Kani formal proofs (cfg kani)

pub mod dpor;

pub use dpor::{
    DporRunner, DporScheduler, DporStats, KiDporScheduler, KiState, LivenessViolation, Operation,
    Schedule, StepRecord, ThreadStatus, TinyBitSet, VectorClock,
};
