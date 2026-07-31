// SPDX-License-Identifier: Apache-2.0
//! The Axiom execution contract.
//!
//! # What this crate owns
//!
//! A DPOR core explores schedules; an *operation source* supplies the operations
//! to explore. This crate holds the vocabulary those two sides speak — and
//! nothing else. It has no engine in it, no scheduler, no oracle, and no
//! transport: it is a leaf crate whose entire content is the contract surface.
//!
//! Concretely it owns two things that used to sit in two different modules of
//! `laplace-interfaces`:
//!
//! - **the alphabet** — [`ThreadId`] and [`ResourceId`], the identities every
//!   operation is expressed in terms of;
//! - **the contract** — [`AxiomOperation`], [`StepOutcome`], [`AxiomThreadSet`],
//!   [`YieldKind`], [`PanicReport`], [`SourceError`], [`DeterminismClass`], and
//!   the [`ExecutionSource`] trait that ties them together.
//!
//! ## Why the alphabet is here and not next door
//!
//! The alphabet is not an incidental import. `ExecutionSource::step` takes a
//! [`ThreadId`] and `AxiomOperation` is meaningless without a [`ResourceId`], so
//! a crate holding the contract without the identities cannot compile on its
//! own — it would have to depend back on the crate it was extracted from, which
//! in turn needs [`DeterminismClass`] for its determinism report. That is a
//! dependency cycle, and Cargo rejects it. The alphabet travels with the
//! contract because it *is* part of the contract.
//!
//! ## Why Apache-2.0
//!
//! [`ExecutionSource`] is a trait **customers implement**. If the contract moved
//! into the private engine workspace it would inherit that workspace's license
//! tier, and customer code would link an Elastic-2.0 surface in order to hand
//! operations to the engine. The contract stays Apache-2.0 for that reason; the
//! decision is recorded in LEP-0035 §2.4(다).
//!
//! ## The ownership rule
//!
//! Adding a public item here adds it to the contract. Ask first whether the new
//! item is something *both* sides need to name — an engine-internal type that
//! merely happens to cross the boundary once belongs on the engine side, behind
//! a conversion.

#![deny(missing_docs)]

mod execution;
mod ids;

pub use execution::{
    AxiomOperation, AxiomThreadSet, DeterminismClass, ExecutionSource, PanicReport, SourceError,
    StepOutcome, YieldKind,
};
pub use ids::{ResourceId, ThreadId};
