#![deny(clippy::all, clippy::pedantic)]

//! Smoke Test: Run all 18 registered harnesses and verify expected verdicts.
//!
//! This is the automated equivalent of `laplace verify --harness all`.
//! Each harness is run through AxiomOracle::run_exhaustive() and the
//! resulting verdict is checked against the harness's declared expectation.

use smoke_test::{get_all_harnesses, run_harness_smoke, verdict_matches_expected};

/// Run all registered harnesses and verify against expected verdicts.
///
/// This test collects all harnesses from Registry and runs each one,
/// comparing actual verdict to expected ("clean" or "bug").
/// A single failing harness doesn't abort the test; instead we collect
/// all failures and assert at the end.
#[test]
fn smoke_all_harnesses_produce_expected_verdicts() {
    let harnesses = get_all_harnesses();
    assert!(!harnesses.is_empty(), "Registry must contain harnesses");

    let mut passed = 0;
    let mut failed = Vec::new();

    for (name, config) in harnesses {
        let verdict = run_harness_smoke(name, 10000, false, ".");
        let verdict_str = verdict_name(&verdict);

        if verdict_matches_expected(&verdict, &config.expected) {
            passed += 1;
            println!(
                "✅ {}: {} (expected: {})",
                name, verdict_str, config.expected
            );
        } else {
            failed.push((
                name.to_string(),
                config.expected.to_string(),
                verdict_str.clone(),
            ));
            println!(
                "❌ {}: {} (expected: {})",
                name, verdict_str, config.expected
            );
        }
    }

    let total = passed + failed.len();
    println!();
    println!("{} / {} harnesses passed", passed, total);

    if !failed.is_empty() {
        eprintln!();
        eprintln!("Failed harnesses:");
        for (name, expected, actual) in &failed {
            eprintln!("  - {} (expected {}, got {})", name, expected, actual);
        }
        panic!("{} harnesses failed", failed.len());
    }
}

/// Helper function to format OracleVerdict for display
fn verdict_name(verdict: &smoke_test::OracleVerdict) -> String {
    use smoke_test::OracleVerdict;
    match verdict {
        OracleVerdict::Clean => "CLEAN".to_string(),
        OracleVerdict::BugFound { .. } => "BUG".to_string(),
    }
}

/// Individual harness tests for faster debugging
macro_rules! define_harness_test {
    ($test_name:ident, $harness_name:literal, $expected:literal) => {
        #[test]
        fn $test_name() {
            let verdict = run_harness_smoke($harness_name, 10000, false, ".");
            assert!(
                verdict_matches_expected(&verdict, $expected),
                "{} should be {}, got {}",
                $harness_name,
                $expected,
                verdict_name(&verdict)
            );
        }
    };
}

define_harness_test!(smoke_harness_template_harness, "template_harness", "clean");
define_harness_test!(
    smoke_harness_time_lamport_ordering,
    "time_lamport_ordering",
    "clean"
);
define_harness_test!(
    smoke_harness_scheduler_liveness_roundrobin,
    "scheduler_liveness_roundrobin",
    "clean"
);
define_harness_test!(
    smoke_harness_telemetry_ring_buffer_concurrent,
    "telemetry_ring_buffer_concurrent",
    "clean"
);
define_harness_test!(
    smoke_harness_telemetry_atomic_increment,
    "telemetry_atomic_increment",
    "clean"
);
define_harness_test!(
    smoke_harness_pool_preemption_fairness,
    "pool_preemption_fairness",
    "clean"
);
define_harness_test!(
    smoke_harness_memory_write_serialization,
    "memory_write_serialization",
    "clean"
);
define_harness_test!(
    smoke_harness_memory_cross_core_visibility,
    "memory_cross_core_visibility",
    "clean"
);
define_harness_test!(
    smoke_harness_memory_buffer_overflow,
    "memory_buffer_overflow",
    "bug"
);
define_harness_test!(
    smoke_harness_resource_starvation_greedy,
    "resource_starvation_greedy",
    "bug"
);
define_harness_test!(
    smoke_harness_resource_fair_independent,
    "resource_fair_independent",
    "clean"
);
define_harness_test!(
    smoke_harness_resource_priority_inversion,
    "resource_priority_inversion",
    "bug"
);
define_harness_test!(
    smoke_harness_entropy_snapshot_determinism,
    "entropy_snapshot_determinism",
    "clean"
);
define_harness_test!(
    smoke_harness_resource_abba_deadlock,
    "resource_abba_deadlock",
    "bug"
);
define_harness_test!(
    smoke_harness_core_resource_pool,
    "core_resource_pool",
    "clean"
);
define_harness_test!(
    smoke_harness_journal_concurrent_log_ordering,
    "journal_concurrent_log_ordering",
    "clean"
);
define_harness_test!(
    smoke_harness_tracing_causality_acyclicity,
    "tracing_causality_acyclicity",
    "bug"
);
define_harness_test!(
    smoke_harness_benchmark_stability_snapshot,
    "benchmark_stability_snapshot",
    "clean"
);
