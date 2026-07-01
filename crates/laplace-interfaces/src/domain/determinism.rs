// SPDX-License-Identifier: Apache-2.0
//! Determinism-declaration report contracts (#89 = M1-PROD-1).
//!
//! These types are the public data model for the determinism-declaration UX.
//! An execution produces a [`DeterminismReport`]: the [`DeterminismClass`] is
//! the *verdict* (owned by the engine / detector A — behavioral divergence),
//! while [`NonDeterminismFinding`]s are the *explanations* (what/where, plus a
//! remedy and a confidence tier). Findings never certify determinism; only a
//! [`Confidence::Confirmed`] finding may justify downgrading the class.
//!
//! The report is emitted to a sidecar (`*.determinism.json`) and rendered as
//! `verify` stdout comments — the `.ard` replay artifact is left bit-exact.

use crate::domain::axiom_execution::DeterminismClass;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Source location where a routed resource (or non-deterministic call) was seen.
///
/// Captured at instrumentation time via `#[track_caller]` so a finding can name
/// the user's call site (`file:line:col`) instead of an opaque resource id.
/// Only compile-time-stable fields are kept; no absolute paths or source
/// snippets (mirrors the `PanicReport` stable-field policy).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SrcLoc {
    /// Source file path as recorded by the compiler (typically workspace-relative).
    pub file: String,
    /// 1-based line number of the construction site.
    pub line: u32,
    /// 1-based column number of the construction site.
    pub col: u32,
}

impl SrcLoc {
    /// Creates a source location from explicit, already-stable fields.
    #[must_use]
    pub fn new(file: impl Into<String>, line: u32, col: u32) -> Self {
        Self {
            file: file.into(),
            line,
            col,
        }
    }

    /// Creates a source location from a `#[track_caller]` panic location.
    #[must_use]
    pub fn from_location(loc: &std::panic::Location<'_>) -> Self {
        Self {
            file: loc.file().to_string(),
            line: loc.line(),
            col: loc.column(),
        }
    }
}

impl fmt::Display for SrcLoc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file, self.line, self.col)
    }
}

/// Category of a non-deterministic input or observation.
///
/// The first six are *sources* that detector B (source scan) can name
/// statically; [`NdKind::DivergenceObserved`] is only ever produced by detector
/// A (the engine) when the same schedule re-executes into a different op-stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NdKind {
    /// Wall-clock / monotonic time read (`Instant::now`, `SystemTime::now`).
    Time,
    /// Randomness draw not seeded through the model entropy source.
    Rng,
    /// Iteration order of an unordered collection (`HashMap`/`HashSet`).
    HashOrder,
    /// Observation of a pointer / allocation address.
    Address,
    /// External I/O side effect (filesystem, network, environment).
    Io,
    /// A thread spawned outside the model's controlled scheduler.
    ExternalThread,
    /// Behavioral divergence proven by the engine (same schedule, different ops).
    DivergenceObserved,
}

impl NdKind {
    /// Stable lowercase identifier used in sidecar JSON and `--message-format`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            NdKind::Time => "time",
            NdKind::Rng => "rng",
            NdKind::HashOrder => "hash_order",
            NdKind::Address => "address",
            NdKind::Io => "io",
            NdKind::ExternalThread => "external_thread",
            NdKind::DivergenceObserved => "divergence_observed",
        }
    }
}

impl fmt::Display for NdKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How much the tool trusts a finding.
///
/// Only [`Confidence::Confirmed`] (behavioral divergence proven by the engine)
/// may downgrade the [`DeterminismClass`]; [`Confidence::Likely`] and
/// [`Confidence::Possible`] are static best-effort warnings and never gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Confidence {
    /// Static best-effort guess (heuristic match on an unknown API shape).
    Possible,
    /// Static match against a known non-deterministic API.
    Likely,
    /// Proven at runtime by observed op-stream divergence (detector A).
    Confirmed,
}

impl Confidence {
    /// Stable lowercase identifier for serialized output.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::Possible => "possible",
            Confidence::Likely => "likely",
            Confidence::Confirmed => "confirmed",
        }
    }

    /// Whether a finding at this confidence may downgrade the class.
    #[must_use]
    pub fn gates(self) -> bool {
        matches!(self, Confidence::Confirmed)
    }
}

impl fmt::Display for Confidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Suggested action to make a finding deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Remedy {
    /// Remove the non-deterministic call entirely.
    Eliminate,
    /// Pin the value with `#[laplace::mock_io(...)]` (behavior-preserving).
    Mock,
    /// Declare the input via `.with_declared_inputs([...])` (accept as scoped).
    Declare,
}

impl Remedy {
    /// Stable lowercase identifier for serialized output.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Remedy::Eliminate => "eliminate",
            Remedy::Mock => "mock",
            Remedy::Declare => "declare",
        }
    }
}

impl fmt::Display for Remedy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single non-deterministic input or observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NonDeterminismFinding {
    /// What kind of non-determinism this is.
    pub kind: NdKind,
    /// Where it was observed, if instrumentation captured a site. `None` means
    /// the honest reach boundary: un-instrumented code (e.g. a dependency's
    /// internals).
    pub location: Option<SrcLoc>,
    /// Human-readable, stable evidence string (no absolute paths / snippets).
    pub evidence: String,
    /// Suggested action to make it deterministic.
    pub remedy: Remedy,
    /// How much the tool trusts this finding.
    pub confidence: Confidence,
}

impl NonDeterminismFinding {
    /// Creates a finding with the given fields.
    #[must_use]
    pub fn new(
        kind: NdKind,
        location: Option<SrcLoc>,
        evidence: impl Into<String>,
        remedy: Remedy,
        confidence: Confidence,
    ) -> Self {
        Self {
            kind,
            location,
            evidence: evidence.into(),
            remedy,
            confidence,
        }
    }
}

/// The determinism verdict plus its explanations for one verification run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeterminismReport {
    /// The class verdict, owned by detector A (the engine).
    pub class: DeterminismClass,
    /// Explanatory findings (never certifying; only `Confirmed` ones gate).
    pub findings: Vec<NonDeterminismFinding>,
}

impl DeterminismReport {
    /// Creates a report with an explicit class and findings.
    #[must_use]
    pub fn new(class: DeterminismClass, findings: Vec<NonDeterminismFinding>) -> Self {
        Self { class, findings }
    }

    /// Whether any finding was proven by the engine (`Confidence::Confirmed`).
    ///
    /// This is the only signal allowed to downgrade the class; source-scan
    /// warnings (`Likely`/`Possible`) must not.
    #[must_use]
    pub fn has_confirmed(&self) -> bool {
        self.findings.iter().any(|f| f.confidence.gates())
    }

    /// Returns the class the report should carry given detector-A evidence:
    /// any `Confirmed` finding forces [`DeterminismClass::BestEffort`];
    /// otherwise the incoming class is preserved (findings only explain).
    #[must_use]
    pub fn resolved_class(&self) -> DeterminismClass {
        if self.has_confirmed() {
            DeterminismClass::BestEffort
        } else {
            self.class
        }
    }
}
