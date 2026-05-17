//! Verdict Engine
//!
//! Synthesizes CPI efficiency, tail-latency stability, error rate, and CPU usage
//! into a single actionable system grade and diagnostic report.
//!
//! # Evaluation Priority
//!
//! Signals are evaluated in strict priority order:
//! 1. Error rate        (F — immediate operational failure)
//! 2. Tail-latency stability (C — P99 spikes)
//! 3. CPU bound / efficiency (B — scale-out required)
//! 4. Perfect system   (S — rock-solid, 10× headroom)
//! 5. Default healthy  (A — normal operation)

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// SystemGrade
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Overall system health grade produced by [`DiagnosticEngine`].
///
/// Variants are declared in ascending quality order so that `derive(Ord)` gives
/// `F < C < B < A < S` — i.e. a higher discriminant means a healthier system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SystemGrade {
    /// F: Failing — error_rate > 5 %
    F,
    /// C: Critical — stability_score < 60.0
    C,
    /// B: Warning — CPU bound (cpi > 5 000 && cpu_usage > 80 %)
    B,
    /// A: Good — healthy but not perfect
    A,
    /// S: Perfect — cpi ≤ 1 000, stability ≥ 95, zero errors
    S,
}

impl std::fmt::Display for SystemGrade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SystemGrade::S => "S",
            SystemGrade::A => "A",
            SystemGrade::B => "B",
            SystemGrade::C => "C",
            SystemGrade::F => "F",
        })
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DiagnosticReport
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Synthesized diagnostic report produced by [`DiagnosticEngine::evaluate`].
///
/// All string fields are `&'static str` to eliminate heap allocation on the
/// hot diagnostic path — all advice strings are compile-time constants.
#[derive(Debug, Clone)]
pub struct DiagnosticReport {
    /// Overall system health grade.
    pub grade: SystemGrade,
    /// Human-readable identification of the primary bottleneck.
    pub primary_bottleneck: &'static str,
    /// Concrete steps to resolve the identified bottleneck.
    pub actionable_advice: &'static str,
}

impl DiagnosticReport {
    #[inline]
    fn new(
        grade: SystemGrade,
        primary_bottleneck: &'static str,
        actionable_advice: &'static str,
    ) -> Self {
        Self {
            grade,
            primary_bottleneck,
            actionable_advice,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DiagnosticEngine
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Synthesizes four telemetry signals into a single system health diagnosis.
pub struct DiagnosticEngine;

impl DiagnosticEngine {
    /// Evaluate system health from four core telemetry signals.
    ///
    /// # Arguments
    ///
    /// - `cpi`             — Cycles Per Instruction score (higher = more efficient)
    /// - `stability_score` — P50/P99 ratio ×100 in [0.0, 100.0] (higher = more stable)
    /// - `error_rate`      — Fraction of requests that errored [0.0, 1.0]
    /// - `cpu_usage`       — CPU utilisation percentage [0.0, 100.0]
    ///
    /// # Returns
    ///
    /// A zero-allocation [`DiagnosticReport`] with grade and actionable advice.
    pub fn evaluate(
        cpi: f64,
        stability_score: f64,
        error_rate: f64,
        cpu_usage: f64,
    ) -> DiagnosticReport {
        if error_rate > 0.05 {
            return DiagnosticReport::new(
                SystemGrade::F,
                "High Error Rate",
                "어플리케이션 로그 및 서킷 브레이커, DB 커넥션 풀을 즉시 확인하라.",
            );
        }

        if stability_score < 60.0 {
            return DiagnosticReport::new(
                SystemGrade::C,
                "High Tail Latency (P99 Spikes)",
                "GC Pause, 락 경합(Lock Contention), 또는 동기적 I/O 블로킹이 발생하고 있다. 비동기 처리 상태를 점검하라.",
            );
        }

        if cpi > 5000.0 && cpu_usage > 80.0 {
            return DiagnosticReport::new(
                SystemGrade::B,
                "CPU Bound / Low Efficiency",
                "CPU 연산 오버헤드가 높다. 루프 최적화 또는 Scale-out이 필요하다.",
            );
        }

        if cpi <= 1000.0 && stability_score >= 95.0 && error_rate == 0.0 {
            return DiagnosticReport::new(
                SystemGrade::S,
                "None",
                "시스템이 극도로 안정적(Rock Solid)이다. 현재 트래픽의 10배 확장을 견딜 준비가 되었다.",
            );
        }

        DiagnosticReport::new(
            SystemGrade::A,
            "None",
            "시스템이 정상적으로 동작 중이다. 모니터링을 유지하고 트래픽 증가에 대비하라.",
        )
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grade_f_on_high_error_rate() {
        let r = DiagnosticEngine::evaluate(2000.0, 90.0, 0.06, 50.0);
        assert_eq!(r.grade, SystemGrade::F);
        assert_eq!(r.primary_bottleneck, "High Error Rate");
    }

    #[test]
    fn test_grade_c_on_unstable_latency() {
        let r = DiagnosticEngine::evaluate(2000.0, 55.0, 0.0, 50.0);
        assert_eq!(r.grade, SystemGrade::C);
        assert_eq!(r.primary_bottleneck, "High Tail Latency (P99 Spikes)");
    }

    #[test]
    fn test_grade_b_on_cpu_bound() {
        let r = DiagnosticEngine::evaluate(6000.0, 75.0, 0.0, 85.0);
        assert_eq!(r.grade, SystemGrade::B);
        assert_eq!(r.primary_bottleneck, "CPU Bound / Low Efficiency");
    }

    #[test]
    fn test_grade_s_perfect_system() {
        let r = DiagnosticEngine::evaluate(800.0, 98.0, 0.0, 30.0);
        assert_eq!(r.grade, SystemGrade::S);
        assert_eq!(r.primary_bottleneck, "None");
    }

    #[test]
    fn test_grade_a_healthy_system() {
        let r = DiagnosticEngine::evaluate(3000.0, 80.0, 0.01, 60.0);
        assert_eq!(r.grade, SystemGrade::A);
    }

    #[test]
    fn test_f_has_priority_over_c() {
        // error_rate > 5% dominates even when stability is already low
        let r = DiagnosticEngine::evaluate(2000.0, 50.0, 0.10, 50.0);
        assert_eq!(r.grade, SystemGrade::F);
    }

    #[test]
    fn test_error_rate_boundary_exact_5pct_not_f() {
        // Boundary: exactly 5% must NOT trigger F (condition is strictly >)
        let r = DiagnosticEngine::evaluate(2000.0, 90.0, 0.05, 50.0);
        assert_ne!(r.grade, SystemGrade::F);
    }

    #[test]
    fn test_stability_boundary_exact_60_not_c() {
        // Boundary: exactly 60.0 must NOT trigger C (condition is strictly <)
        let r = DiagnosticEngine::evaluate(2000.0, 60.0, 0.0, 50.0);
        assert_ne!(r.grade, SystemGrade::C);
    }

    #[test]
    fn test_s_requires_zero_error_rate() {
        // Even tiny non-zero error_rate prevents S grade
        let r = DiagnosticEngine::evaluate(500.0, 99.0, 0.001, 20.0);
        assert_ne!(r.grade, SystemGrade::S);
    }

    #[test]
    fn test_b_requires_both_high_cpi_and_cpu() {
        // High CPI alone (cpu_usage ≤ 80) does NOT trigger B
        let r = DiagnosticEngine::evaluate(6000.0, 75.0, 0.0, 70.0);
        assert_ne!(r.grade, SystemGrade::B);
    }

    #[test]
    fn test_system_grade_ordering() {
        assert!(SystemGrade::F < SystemGrade::C);
        assert!(SystemGrade::C < SystemGrade::B);
        assert!(SystemGrade::B < SystemGrade::A);
        assert!(SystemGrade::A < SystemGrade::S);
    }
}
