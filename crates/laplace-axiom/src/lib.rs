// SPDX-License-Identifier: Apache-2.0
#![deny(clippy::all)]

//! Laplace Axiom: verification contracts, simulation, and oracle interface.

pub mod dpor {
    pub use laplace_dpor::dpor::*;
    pub use laplace_dpor::{
        DporScheduler, DporStats, Operation, StepRecord, TinyBitSet, VectorClock,
    };
    pub use laplace_ki_dpor::{
        DporRunner, KiDporScheduler, KiState, LivenessViolation, Schedule, ThreadStatus,
    };
}

pub use dpor::{
    DporRunner, DporScheduler, DporStats, KiDporScheduler, KiState, LivenessViolation, Operation,
    Schedule, StepRecord, ThreadStatus, TinyBitSet, VectorClock,
};

pub mod simulation;

pub mod infrastructure;

/// Axiom Oracle — exhaustive DPOR judgment engine with SMT bridge and ARD dump.
pub mod oracle;
pub mod session;
pub use session::{ExplorationStats, VerificationConfig, VerificationSession};

// Re-export probe_listener at the previous path for backwards compatibility.
#[cfg(all(feature = "twin", feature = "verification"))]
pub use infrastructure::probe_listener;
