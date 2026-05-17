// SPDX-License-Identifier: Apache-2.0
//! Axiom Oracle — Right Brain of the Axiom Verification System
//!
//! The Oracle is the exhaustive judgment engine.  Where Vanguard performs
//! shallow, fast checks, the Oracle performs **complete** state-space
//! exploration using DPOR + optional SMT (Z3) symbolic constraints, and
//! issues an irrefutable [`OracleVerdict`] for every target it inspects.
//!
//! # Architecture
//!
//! ```text
//!           ┌──────────────────────────────────────────────┐
//!           │                 AxiomOracle                  │
//!           │  ┌────────────┐    ┌───────────────────────┐ │
//!           │  │  Exhaustive│    │      SmtBridge        │ │
//!           │  │  DPOR Loop │───▶│  (symbolic execution) │ │
//!           │  └─────┬──────┘    └───────────────────────┘ │
//!           │        │ violation detected                   │
//!           │        ▼                                      │
//!           │  ┌────────────────────┐                      │
//!           │  │   VerdictEngine    │  → .ard dump         │
//!           │  └────────────────────┘                      │
//!           └──────────────────────────────────────────────┘
//! ```
//!
//! # Feature gating
//!
//! The Oracle requires `feature = "twin"` (DPOR + simulation).
//! `feature = "verification"` enables the live [`run_exhaustive`](AxiomOracle::run_exhaustive)
//! method.  The SMT bridge is always compiled; only the stub solver ships by default.

#![cfg(feature = "twin")]

#[cfg(feature = "verification")]
use laplace_core::domain::resource::{ResourceId, ThreadId};
use laplace_interfaces::AxiomConfig;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

#[cfg(feature = "verification")]
use crate::simulation::TwinSimulator;
#[cfg(feature = "verification")]
use laplace_dpor::Operation;
#[cfg(feature = "verification")]
use laplace_ki_dpor::{DporRunner, KiDporScheduler, LivenessViolation, Schedule};

#[cfg(feature = "engine")]
mod engine;
#[cfg(feature = "engine")]
pub use engine::{load_ard, save_ard};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// SMT Bridge — Z3 abstraction layer
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A symbolic constraint expressed as an SMT-LIB2 s-expression.
///
/// Any conforming solver back-end (Z3, CVC5, …) can consume these without
/// translation.  Example: `"(assert (>= ticket_count 0))"`.
#[derive(Debug, Clone)]
pub struct SmtConstraint {
    /// SMT-LIB2 expression string.
    pub smt_lib2: String,
}

impl SmtConstraint {
    /// Wrap an SMT-LIB2 expression.
    pub fn new(smt_lib2: impl Into<String>) -> Self {
        Self {
            smt_lib2: smt_lib2.into(),
        }
    }
}

/// Decision returned by an SMT solver query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmtResult {
    /// Constraint set is satisfiable — a counter-example model exists.
    Satisfiable,
    /// Constraint set is unsatisfiable — the property holds universally.
    Unsatisfiable,
    /// Solver could not decide within its resource limits.
    Unknown,
}

/// Satisfying assignment produced when [`SmtResult::Satisfiable`].
#[derive(Debug, Clone, Default)]
pub struct SmtModel {
    /// `(variable_name, value)` pairs from the model.
    pub assignments: Vec<(String, String)>,
}

/// Trait abstracting any SMT solver back-end.
///
/// Implement this trait to plug in Z3 (via `z3-rs`), CVC5, or any other
/// solver.  Until a real solver is linked, use [`StubSmtSolver`].
pub trait SmtSolver {
    /// Assert a constraint into the solver's current context.
    fn assert(&mut self, constraint: SmtConstraint);
    /// Run the solver and return a satisfiability decision.
    fn check(&self) -> SmtResult;
    /// Produce a concrete model if the last [`check`](Self::check) was `Satisfiable`.
    fn model(&self) -> Option<SmtModel>;
    /// Clear all assertions to start a fresh query.
    fn reset(&mut self);
}

/// No-op SMT solver — always returns [`SmtResult::Unknown`].
///
/// Used as the default back-end so that the Oracle pipeline compiles and
/// runs without a native Z3 installation.  Replace with a real solver
/// implementation once the native library is available.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "30_Axiom_Oracle",
        link = "LEP-0011-laplace-axiom-oracle_forensics_and_bmc"
    )
)]
#[derive(Debug, Default)]
pub struct StubSmtSolver {
    constraints: Vec<SmtConstraint>,
}

impl SmtSolver for StubSmtSolver {
    fn assert(&mut self, c: SmtConstraint) {
        self.constraints.push(c);
    }
    fn check(&self) -> SmtResult {
        SmtResult::Unknown
    }
    fn model(&self) -> Option<SmtModel> {
        None
    }
    fn reset(&mut self) {
        self.constraints.clear();
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Oracle configuration
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Configuration for [`AxiomOracle`].
#[derive(Debug, Clone)]
pub struct OracleConfig {
    /// Number of virtual threads modelled during DPOR exploration.  Default: 2.
    pub num_threads: usize,

    /// Number of virtual resources modelled during DPOR exploration.  Default: 2.
    pub num_resources: usize,

    /// Maximum DPOR exploration depth.
    ///
    /// The Oracle's default is intentionally much higher than Vanguard's
    /// shallow bound to guarantee exhaustive coverage.
    pub max_depth: usize,

    /// Master seed stored verbatim in every `.ard` header for replay.
    pub axiom_seed: u64,

    /// Directory for `.ard` output files.  Defaults to `"."`.
    pub output_dir: String,

    /// Whether to write `.ard` forensic files to disk on violation.
    ///
    /// Set to `false` for harnesses where a bug is *expected* (`expected = "bug"`)
    /// to avoid creating noise files in CI pipelines.  Defaults to `true`.
    pub write_ard: bool,
}

impl Default for OracleConfig {
    fn default() -> Self {
        Self {
            num_threads: 2,
            num_resources: 2,
            max_depth: 10_000,
            axiom_seed: AxiomConfig::default().default_seed,
            output_dir: ".".to_string(),
            write_ard: true,
        }
    }
}

impl OracleConfig {
    /// Convenience constructor that overrides only the seed.
    pub fn with_seed(seed: u64) -> Self {
        Self {
            axiom_seed: seed,
            ..Self::default()
        }
    }

    /// Build an `OracleConfig` from a `laplace_interfaces::AxiomConfig`.
    ///
    /// `num_threads` and `num_resources` are taken from `axiom_cfg.max_threads`
    /// so that the DPOR scheduler respects the global thread ceiling.
    pub fn from_axiom_config(axiom_cfg: &AxiomConfig) -> Self {
        Self {
            num_threads: axiom_cfg.max_threads as usize,
            num_resources: axiom_cfg.max_threads as usize, // symmetric default
            max_depth: axiom_cfg.max_depth as usize,
            axiom_seed: axiom_cfg.default_seed,
            output_dir: ".".to_string(),
            write_ard: true,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// OracleVerdict
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// The definitive judgment produced by [`AxiomOracle`].
#[derive(Debug, Clone)]
pub enum OracleVerdict {
    /// Exhaustive search completed without any violation.
    Clean,

    /// A concurrency bug was confirmed; an `.ard` file was written to disk.
    BugFound {
        /// Absolute or relative path to the generated `.ard` forensic report.
        ard_path: String,
        /// Human-readable description of the violation.
        description: String,
    },
}

// VerdictEngine is now in engine.rs (feature-gated behind "engine")

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DporRunnerExt — Simulator integration trait
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(feature = "verification")]
pub trait DporRunnerExt {
    /// Run the DPOR exploration loop with an additional **invariant checker**.
    fn run_with_invariants<F, I>(
        &mut self,
        simulator: &mut TwinSimulator,
        max_steps: usize,
        op_provider: F,
        invariant_checker: I,
    ) -> Option<Schedule>
    where
        F: FnMut(ThreadId, usize) -> Option<(Operation, ResourceId)>,
        I: FnMut(&mut TwinSimulator) -> Option<String>;
}

#[cfg(feature = "verification")]
impl DporRunnerExt for DporRunner {
    fn run_with_invariants<F, I>(
        &mut self,
        simulator: &mut TwinSimulator,
        max_steps: usize,
        mut op_provider: F,
        mut invariant_checker: I,
    ) -> Option<Schedule>
    where
        F: FnMut(ThreadId, usize) -> Option<(Operation, ResourceId)>,
        I: FnMut(&mut TwinSimulator) -> Option<String>,
    {
        use laplace_core::domain::telemetry::{GlobalTelemetry, TelemetryEvent};

        let scheduler = self.scheduler_mut();
        let mut last_explored: usize = 0;

        for _ in 0..max_steps {
            scheduler.advance();
            scheduler.expand_current(&mut op_provider);

            // Push DPOR progress to GlobalTelemetry (one inc per state expanded).
            let curr = scheduler.stats().explored_states;
            if curr > last_explored {
                GlobalTelemetry::metrics().inc_explored_states();
                last_explored = curr;
            }

            if let Some(schedule) = scheduler.extract_schedule() {
                let desc = schedule
                    .violation
                    .as_ref()
                    .map(|v| format!("{v:?}"))
                    .unwrap_or_else(|| "UnknownViolation".to_string());
                GlobalTelemetry::events().push(TelemetryEvent::AxiomViolation(desc));
                return Some(schedule);
            }

            if scheduler.is_complete() {
                break;
            }

            let _ = simulator.step();

            // ── Invariant check ──────────────────────────────────────────────
            if let Some(msg) = invariant_checker(simulator) {
                scheduler.set_violation(LivenessViolation::InvariantViolation {
                    description: msg,
                });
                if let Some(schedule) = scheduler.extract_schedule() {
                    let desc = schedule
                        .violation
                        .as_ref()
                        .map(|v| format!("{v:?}"))
                        .unwrap_or_else(|| "UnknownViolation".to_string());
                    GlobalTelemetry::events().push(TelemetryEvent::AxiomViolation(desc));
                    return Some(schedule);
                }
            }
        }

        let result = scheduler.extract_schedule();
        if result.is_some() {
            let desc = result
                .as_ref()
                .and_then(|s| s.violation.as_ref().map(|v| format!("{v:?}")))
                .unwrap_or_else(|| "UnknownViolation".to_string());
            GlobalTelemetry::events().push(TelemetryEvent::AxiomViolation(desc));
        } else {
            GlobalTelemetry::events().push(TelemetryEvent::AxiomClean);
        }
        result
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// AxiomOracle — public entry point
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// The Axiom Oracle engine.
///
/// Performs exhaustive DPOR state-space exploration, optionally augmented with
/// SMT-based symbolic reasoning.  On first violation it halts, dumps an `.ard`
/// forensic file, and returns [`OracleVerdict::BugFound`].
pub struct AxiomOracle {
    #[allow(dead_code)]
    config: OracleConfig,
}

impl AxiomOracle {
    /// Create an Oracle with explicit configuration.
    pub fn new(config: OracleConfig) -> Self {
        Self { config }
    }

    /// Create an Oracle with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(OracleConfig::default())
    }

    /// Run exhaustive DPOR exploration against `target_id`.
    ///
    /// # Parameters
    ///
    /// * `target_id` — Label for the component under verification (stored in ARD header).
    /// * `simulator` — [`TwinSimulator`] to step alongside DPOR exploration.
    /// * `max_steps` — Step budget; pass `0` to use [`OracleConfig::max_depth`].
    /// * `op_provider` — Maps `(thread, pc)` to the next `(Operation, ResourceId)`,
    ///   or `None` if the thread has terminated.
    /// * `invariant_checker` — Returns `Some(description)` on invariant violation,
    ///   `None` to continue exploration.
    ///
    /// # Returns
    ///
    /// [`OracleVerdict::Clean`] if exhaustive search found no bugs, or
    /// [`OracleVerdict::BugFound`] with an `.ard` path and violation description.
    #[cfg(feature = "verification")]
    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(
            layer = "30_Axiom_Oracle",
            link = "LEP-0011-laplace-axiom-oracle_forensics_and_bmc"
        )
    )]
    pub fn run_exhaustive<F, I>(
        &self,
        target_id: &str,
        simulator: &mut TwinSimulator,
        max_steps: usize,
        op_provider: F,
        invariant_checker: I,
    ) -> OracleVerdict
    where
        F: FnMut(ThreadId, usize) -> Option<(Operation, ResourceId)>,
        I: FnMut(&mut TwinSimulator) -> Option<String>,
    {
        let depth = if max_steps == 0 {
            self.config.max_depth
        } else {
            max_steps
        };

        tracing::info!(
            target = target_id,
            depth,
            seed = self.config.axiom_seed,
            "AxiomOracle: starting exhaustive DPOR sweep"
        );

        // Build an AxiomConfig from OracleConfig fields so KiDporScheduler
        // picks up max_starvation_limit and max_danger from the global config.
        let axiom_cfg = AxiomConfig {
            max_threads: self.config.num_threads as u32,
            max_depth: self.config.max_depth as u32,
            default_seed: self.config.axiom_seed,
            ..AxiomConfig::default()
        };
        let scheduler = KiDporScheduler::with_config(
            self.config.num_threads,
            self.config.num_resources,
            &axiom_cfg,
        );
        let mut runner = DporRunner::new(scheduler);
        let result = runner.run_with_invariants(simulator, depth, op_provider, invariant_checker);

        match result {
            None => {
                tracing::info!(
                    target = target_id,
                    "AxiomOracle: CLEAN — no violations found"
                );
                OracleVerdict::Clean
            }
            Some(ref _schedule) => {
                #[cfg(feature = "engine")]
                let schedule = _schedule;
                #[cfg(feature = "engine")]
                {
                    let engine = engine::VerdictEngine::new(self.config.clone());
                    engine.dump(target_id, schedule)
                }
                #[cfg(not(feature = "engine"))]
                {
                    tracing::warn!("AxiomOracle: engine feature disabled; returning stub verdict");
                    OracleVerdict::BugFound {
                        ard_path: "<engine-not-included>".to_string(),
                        description: "Violation detected but engine feature is disabled"
                            .to_string(),
                    }
                }
            }
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stub_smt_solver() {
        let mut solver = StubSmtSolver::default();
        solver.assert(SmtConstraint::new("(assert (>= x 0))"));
        assert_eq!(solver.check(), SmtResult::Unknown);
        assert!(solver.model().is_none());
        solver.reset();
    }

    #[test]
    fn test_oracle_config_default() {
        let cfg = OracleConfig::default();
        assert_eq!(cfg.num_threads, 2);
        assert!(cfg.max_depth >= 1000);
    }

    #[cfg(feature = "verification")]
    #[test]
    fn test_oracle_detects_deadlock() {
        use crate::simulation::TwinSimulatorBuilder;
        use laplace_core::domain::memory::{Address, CoreId, Value};

        let oracle = AxiomOracle::new(OracleConfig {
            max_depth: 200,
            output_dir: std::env::temp_dir().to_string_lossy().into_owned(),
            ..OracleConfig::default()
        });

        let mut sim = TwinSimulatorBuilder::new()
            .cores(2)
            .scheduler_threads(2)
            .finalize()
            .build();

        sim.memory_mut()
            .write(CoreId::new(0), Address::new(0), Value::new(1))
            .unwrap();

        let verdict = oracle.run_exhaustive(
            "deadlock_test",
            &mut sim,
            200,
            |_t, _pc| Some((Operation::Request, ResourceId::new(0))),
            |_sim| None,
        );

        assert!(
            matches!(verdict, OracleVerdict::BugFound { .. }),
            "Oracle must detect the always-request deadlock"
        );
    }
}
