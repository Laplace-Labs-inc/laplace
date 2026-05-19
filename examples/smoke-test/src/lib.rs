#![deny(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

//! Smoke Test — End-to-end verification of Laplace core product pipeline.
//!
//! This crate provides library-level testing of:
//! 1. **Verification**: All 18 registered harnesses via `AxiomOracle`
//! 2. **Forensics**: ARD (Axiom Report Dump) file generation and lifecycle
//! 3. **Depth Control**: `max_depth` parameter enforcement

pub use laplace_axiom::oracle::OracleVerdict;

use laplace_axiom::oracle::{AxiomOracle, OracleConfig};
use laplace_axiom::simulation::TwinSimulatorBuilder;
use laplace_core::domain::memory::{Address, CoreId, Value};
use laplace_harness::registry::{HarnessConfig, Registry};

/// Run a single harness and return its verdict.
///
/// This function mirrors the pattern from `laplace-cli/src/commands/verify.rs:run_harness()`.
/// It creates a `TwinSimulator`, seeds its memory, and runs the harness through `AxiomOracle`.
///
/// # Parameters
/// - `name`: Harness name (e.g., "`template_harness`", "`resource_abba_deadlock`")
/// - `max_depth`: DPOR exploration budget
/// - `write_ard`: Whether to write .ard files on bug discovery
/// - `output_dir`: Directory for .ard output
///
/// # Returns
/// The Oracle verdict after exhaustive DPOR sweep.
///
/// # Panics
/// Panics if harness is not found in registry.
#[must_use]
pub fn run_harness_smoke(
    name: &str,
    max_depth: usize,
    write_ard: bool,
    output_dir: &str,
) -> OracleVerdict {
    let resolved =
        Registry::get(name).unwrap_or_else(|_| panic!("Harness '{name}' not found in registry"));

    // Create TwinSimulator with appropriate thread and resource counts
    let mut simulator = TwinSimulatorBuilder::new()
        .cores(resolved.num_threads)
        .scheduler_threads(resolved.num_threads)
        .finalize()
        .build();

    // Seed memory — this is required by AxiomOracle to establish initial state
    for i in 0..resolved.num_threads {
        simulator.run_until_idle();
        simulator
            .memory_mut()
            .write(CoreId::new(0), Address::new(i), Value::new(i as u64 + 1))
            .expect("Simulator seed write failed");
    }
    simulator.run_until_idle();

    // Create Oracle with explicit config matching harness expectations
    let oracle = AxiomOracle::new(OracleConfig {
        num_threads: resolved.num_threads,
        num_resources: resolved.num_resources,
        max_depth,
        output_dir: output_dir.to_string(),
        write_ard,
        ..OracleConfig::default()
    });

    // Run exhaustive DPOR sweep
    oracle.run_exhaustive(
        &format!("harness::{name}"),
        &mut simulator,
        max_depth,
        resolved.op_provider,
        |_sim| None,
    )
}

/// Collect all harness configs from registry.
#[must_use]
pub fn get_all_harnesses() -> Vec<(&'static str, HarnessConfig)> {
    Registry::get_all()
}

/// Convert expected verdict string to `OracleVerdict` for comparison.
#[must_use]
pub fn parse_expected(expected: &str) -> Option<&str> {
    match expected {
        "clean" | "bug" => Some(expected),
        _ => None,
    }
}

/// Check if verdict matches expected result.
#[must_use]
pub fn verdict_matches_expected(verdict: &OracleVerdict, expected: &str) -> bool {
    matches!(
        (verdict, expected),
        (OracleVerdict::Clean, "clean") | (OracleVerdict::BugFound { .. }, "bug")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_smoke_registry_accessible() {
        let harnesses = get_all_harnesses();
        assert!(!harnesses.is_empty(), "Registry must contain harnesses");
    }

    #[test]
    fn test_smoke_simple_harness() {
        // Run a simple CLEAN harness to verify smoke test infrastructure works
        let verdict = run_harness_smoke("template_harness", 1000, false, ".");
        assert!(
            verdict_matches_expected(&verdict, "clean"),
            "template_harness should be CLEAN, got {verdict:?}"
        );
    }
}
