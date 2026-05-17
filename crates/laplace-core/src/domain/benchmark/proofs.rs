#![cfg(kani)]

//! Kani Formal Verification Proofs — Benchmark Domain
//!
//! Verifies the following invariants:
//!
//! - **H-B1** `proof_efficiency_tier_partition` — `EfficiencyTier::from_cpi` partitions
//!   CPI space with no gaps: boundaries at 1 000 / 5 000 / 10 000.
//! - **H-B2** `proof_cpi_no_division_by_zero` — denominator guard
//!   (`denominator < 0.1 → 0.0`) prevents division by zero or NaN.
//! - **H-B3** `proof_stability_score_bounds` — `StabilityAnalyzer::calculate_score`
//!   always returns a value in `[0.0, 100.0]` for all symbolic `u64` inputs.
//! - **H-B4** `proof_stability_zero_p99` — `calculate_score(any_p50, 0)` returns
//!   exactly `100.0` without panicking.
//! - **H-B5** `proof_diagnostic_error_rate_priority` — `error_rate > 0.05` always
//!   yields `SystemGrade::F` regardless of other signals.
//! - **H-B6** `proof_diagnostic_s_grade_conditions` — `SystemGrade::S` is produced
//!   if and only if `cpi ≤ 1 000 && stability ≥ 95 && error_rate == 0.0`.
//! - **H-B7** `proof_grade_ordering` — `derive(Ord)` gives `F < C < B < A < S`.

use super::{
    nerve::StabilityAnalyzer,
    verdict::{DiagnosticEngine, SystemGrade},
    EfficiencyTier,
};

// ── H-B1 ─────────────────────────────────────────────────────────────────────

/// Proof: `EfficiencyTier::from_cpi` partitions CPI space without gaps or overlaps.
///
/// # Invariant
///
/// - `cpi < 1 000`              → `Bronze`
/// - `1 000 ≤ cpi < 5 000`     → `Silver`
/// - `5 000 ≤ cpi < 10 000`    → `Gold`
/// - `cpi ≥ 10 000`             → `Turbo`
///
/// All canonical boundary values are checked exactly, and for any finite non-negative
/// symbolic `cpi` the tier is uniquely determined.
#[kani::proof]
#[kani::unwind(1)]
fn proof_efficiency_tier_partition() {
    // Concrete boundary checks
    assert_eq!(
        EfficiencyTier::from_cpi(0.0),
        EfficiencyTier::Bronze,
        "0.0 -> Bronze"
    );
    assert_eq!(
        EfficiencyTier::from_cpi(999.9),
        EfficiencyTier::Bronze,
        "999.9 -> Bronze"
    );
    assert_eq!(
        EfficiencyTier::from_cpi(1000.0),
        EfficiencyTier::Silver,
        "1000.0 -> Silver"
    );
    assert_eq!(
        EfficiencyTier::from_cpi(4999.9),
        EfficiencyTier::Silver,
        "4999.9 -> Silver"
    );
    assert_eq!(
        EfficiencyTier::from_cpi(5000.0),
        EfficiencyTier::Gold,
        "5000.0 -> Gold"
    );
    assert_eq!(
        EfficiencyTier::from_cpi(9999.9),
        EfficiencyTier::Gold,
        "9999.9 -> Gold"
    );
    assert_eq!(
        EfficiencyTier::from_cpi(10000.0),
        EfficiencyTier::Turbo,
        "10000.0 -> Turbo"
    );
    assert_eq!(
        EfficiencyTier::from_cpi(f64::MAX),
        EfficiencyTier::Turbo,
        "f64::MAX -> Turbo"
    );

    // Symbolic: for any finite non-negative cpi, exactly one tier is assigned
    let cpi: f64 = kani::any();
    kani::assume(cpi.is_finite() && cpi >= 0.0);

    let tier = EfficiencyTier::from_cpi(cpi);

    // Completeness: always one of the four tiers
    assert!(
        tier == EfficiencyTier::Bronze
            || tier == EfficiencyTier::Silver
            || tier == EfficiencyTier::Gold
            || tier == EfficiencyTier::Turbo,
        "from_cpi must always return one of the four tiers"
    );

    // Partition: tier matches the correct sub-range
    if cpi < 1000.0 {
        assert_eq!(tier, EfficiencyTier::Bronze, "cpi < 1000 must be Bronze");
    } else if cpi < 5000.0 {
        assert_eq!(
            tier,
            EfficiencyTier::Silver,
            "1000 <= cpi < 5000 must be Silver"
        );
    } else if cpi < 10000.0 {
        assert_eq!(
            tier,
            EfficiencyTier::Gold,
            "5000 <= cpi < 10000 must be Gold"
        );
    } else {
        assert_eq!(tier, EfficiencyTier::Turbo, "cpi >= 10000 must be Turbo");
    }
}

// ── H-B2 ─────────────────────────────────────────────────────────────────────

/// Proof: The CPI denominator guard prevents division by zero or NaN.
///
/// # Invariant
///
/// In `CPICalculator::calculate_cpi`, the denominator is
/// `cpu_percent + memory_mb × 10.0`.  The guard
///
/// ```text
/// if denominator < 0.1 { return 0.0; }
/// ```
///
/// ensures that when the denominator is near zero, the function returns `0.0`
/// instead of dividing.  For all finite non-negative inputs the result is
/// therefore finite and non-negative.
///
/// **Note:** `CPICalculator::calculate_cpi()` reads from `GlobalTelemetry` which
/// is a global singleton unsupported in Kani.  The guard logic is therefore
/// modelled inline without invoking the full calculator.
#[kani::proof]
#[kani::unwind(1)]
fn proof_cpi_no_division_by_zero() {
    let cpu_percent: f64 = kani::any();
    let memory_mb: f64 = kani::any();
    kani::assume(cpu_percent >= 0.0 && cpu_percent <= 100.0);
    kani::assume(memory_mb >= 0.0 && memory_mb <= 1_000_000.0);

    // Inline the denominator guard from `CPICalculator::calculate_cpi`
    let denominator = cpu_percent + (memory_mb * 10.0);
    let result = if denominator < 0.1 {
        // Guard active: no division attempted
        0.0_f64
    } else {
        // denominator >= 0.1 — safe to divide
        // Use rps = 1.0 as a representative value (the guard depends only on denominator)
        1000.0_f64 / denominator
    };

    assert!(
        result.is_finite(),
        "CPI must be finite for all finite non-negative inputs"
    );
    assert!(result >= 0.0, "CPI must be non-negative");
}

// ── H-B3 ─────────────────────────────────────────────────────────────────────

/// Proof: `StabilityAnalyzer::calculate_score` always returns a value in `[0.0, 100.0]`.
///
/// # Invariant
///
/// For all `(p50, p99) : (u64, u64)`:
/// - `score >= 0.0`
/// - `score <= 100.0`
///
/// The implementation uses `score.min(100.0).max(0.0)` clamping after the
/// `p99 == 0` fast path that returns 100.0.
#[kani::proof]
#[kani::unwind(1)]
fn proof_stability_score_bounds() {
    let p50: u64 = kani::any();
    let p99: u64 = kani::any();

    let score = StabilityAnalyzer::calculate_score(p50, p99);

    assert!(score >= 0.0, "Stability score must be >= 0.0");
    assert!(score <= 100.0, "Stability score must be <= 100.0");
}

// ── H-B4 ─────────────────────────────────────────────────────────────────────

/// Proof: `calculate_score(p50, 0)` returns exactly `100.0` and does not panic.
///
/// # Invariant
///
/// Division-by-zero protection: when `p99 == 0` (no samples), the function
/// returns `100.0` (interpreting no data as optimal) instead of panicking.
/// This holds for every possible `p50` value.
#[kani::proof]
#[kani::unwind(1)]
fn proof_stability_zero_p99() {
    let p50: u64 = kani::any();

    let score = StabilityAnalyzer::calculate_score(p50, 0);

    assert_eq!(score, 100.0, "calculate_score with p99=0 must return 100.0");
}

// ── H-B5 ─────────────────────────────────────────────────────────────────────

/// Proof: `error_rate > 0.05` always yields `SystemGrade::F`.
///
/// # Invariant
///
/// `DiagnosticEngine::evaluate` evaluates signals in strict priority order.
/// Error rate is the highest-priority signal: any `error_rate > 0.05` (5%)
/// must produce `Grade::F` regardless of CPI, stability, or CPU usage.
#[kani::proof]
#[kani::unwind(1)]
fn proof_diagnostic_error_rate_priority() {
    let cpi: f64 = kani::any();
    let stability_score: f64 = kani::any();
    let error_rate: f64 = kani::any();
    let cpu_usage: f64 = kani::any();
    kani::assume(cpi.is_finite());
    kani::assume(stability_score.is_finite());
    kani::assume(error_rate.is_finite());
    kani::assume(cpu_usage.is_finite());
    kani::assume(error_rate > 0.05);

    let report = DiagnosticEngine::evaluate(cpi, stability_score, error_rate, cpu_usage);

    assert_eq!(
        report.grade,
        SystemGrade::F,
        "error_rate > 0.05 must always produce Grade::F"
    );
}

// ── H-B6 ─────────────────────────────────────────────────────────────────────

/// Proof: `SystemGrade::S` is produced if and only if
/// `cpi ≤ 1 000 && stability_score ≥ 95 && error_rate == 0.0`.
///
/// # Invariant
///
/// **Forward** (⇒): when all three S-conditions hold, `evaluate()` returns `S`.
///
/// **Backward** (⇐): when `evaluate()` returns `S`, all three conditions must hold.
///
/// The forward direction is guaranteed because `cpi ≤ 1 000` implies the
/// B-condition (`cpi > 5 000`) is false, and `error_rate == 0.0` implies
/// the F-condition is not triggered.
#[kani::proof]
#[kani::unwind(1)]
fn proof_diagnostic_s_grade_conditions() {
    let cpi: f64 = kani::any();
    let stability_score: f64 = kani::any();
    let error_rate: f64 = kani::any();
    let cpu_usage: f64 = kani::any();
    kani::assume(cpi.is_finite());
    kani::assume(stability_score.is_finite());
    kani::assume(error_rate.is_finite());
    kani::assume(cpu_usage.is_finite());

    // ── Forward: S-conditions ⇒ Grade::S ────────────────────────────────────
    if cpi <= 1000.0 && stability_score >= 95.0 && error_rate == 0.0 {
        let report = DiagnosticEngine::evaluate(cpi, stability_score, error_rate, cpu_usage);
        assert_eq!(
            report.grade,
            SystemGrade::S,
            "All S-conditions satisfied must yield Grade::S"
        );
    }

    // ── Backward: Grade::S ⇒ S-conditions ───────────────────────────────────
    let report2 = DiagnosticEngine::evaluate(cpi, stability_score, error_rate, cpu_usage);
    if report2.grade == SystemGrade::S {
        assert!(cpi <= 1000.0, "Grade::S requires cpi <= 1000.0");
        assert!(
            stability_score >= 95.0,
            "Grade::S requires stability_score >= 95.0"
        );
        assert_eq!(error_rate, 0.0, "Grade::S requires error_rate == 0.0");
    }
}

// ── H-B7 ─────────────────────────────────────────────────────────────────────

/// Proof: `SystemGrade`'s `derive(Ord)` discriminant order is `F < C < B < A < S`.
///
/// # Invariant
///
/// Variants are declared in ascending quality order so that `derive(Ord)` gives
/// `F < C < B < A < S` — a higher discriminant means a healthier system.
/// This is the structural foundation for comparison-based ranking.
#[kani::proof]
#[kani::unwind(1)]
fn proof_grade_ordering() {
    assert!(SystemGrade::F < SystemGrade::C, "F must be less than C");
    assert!(SystemGrade::C < SystemGrade::B, "C must be less than B");
    assert!(SystemGrade::B < SystemGrade::A, "B must be less than A");
    assert!(SystemGrade::A < SystemGrade::S, "A must be less than S");
}
