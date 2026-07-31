// SPDX-License-Identifier: Apache-2.0
//! v1 compatibility surface for the Axiom execution contract.
//!
//! The contract itself moved to the `laplace-axiom-contract` crate, which is a
//! leaf: it owns the eight contract symbols plus the `ThreadId`/`ResourceId`
//! alphabet they are written in. This module is what is left behind so that
//! existing callers of `laplace_interfaces::domain::axiom_execution::*` and
//! `laplace_interfaces::*` keep compiling while their imports are repointed.
//!
//! # This module is scheduled for deletion
//!
//! It is deleted in **slice ⑤ of the G9 window (LEP-0035 §4)**, together with
//! the two re-export sites in `domain/mod.rs` and `lib.rs`, and the window does
//! not close while it still exists. The deadline is not decoration: this repo's
//! most common failure is a second copy of a truth that no consumer grep finds
//! (LEP-0035 §2.3 lists three of them), and a compatibility re-export that
//! outlives its migration is exactly that shape — `laplace-core` alone forwards
//! `laplace_interfaces` in 21 places, so a caller can reach v1 without ever
//! naming this module.
//!
//! Do not add anything here. New contract items go in `laplace-axiom-contract`.

pub use laplace_axiom_contract::{
    AxiomOperation, AxiomThreadSet, DeterminismClass, ExecutionSource, PanicReport, SourceError,
    StepOutcome, YieldKind,
};
