// SPDX-License-Identifier: Apache-2.0
//! Benchmark Report Builder (Phase 2.3 - Step 8)
//!
//! Aggregates engine metrics, resource usage, and efficiency scores into a
//! human-readable Markdown report containing:
//! - Executive summary (high-level pass/fail signal)
//! - Request throughput and error rate
//! - Tail-latency percentiles (P99) — when `verification` feature is active
//! - CPU and memory utilization
//! - CPI score and efficiency tier

use crate::domain::benchmark::{EfficiencyTier, ResourceMetrics};
use crate::domain::telemetry::EngineMetrics;

// ─── BenchmarkReportBuilder ───────────────────────────────────────────────────

/// Builds a Markdown benchmark report from engine and resource metrics.
///
/// # Usage
///
/// ```rust
/// # use laplace_core::domain::benchmark::report::BenchmarkReportBuilder;
/// # use laplace_core::domain::benchmark::{EfficiencyTier, ResourceMetrics};
/// # use laplace_core::domain::telemetry::EngineMetrics;
/// let metrics = EngineMetrics::new();
/// metrics.inc_requests();
/// let resources = ResourceMetrics::new(35.0, 256.0);
/// let cpi = 4500.0;
/// let tier = EfficiencyTier::from_cpi(cpi);
/// let report = BenchmarkReportBuilder::new(&metrics, resources, cpi, tier, 10.0)
///     .scenario_name("login_flow")
///     .build();
/// assert!(report.contains("# Laplace Benchmark Report"));
/// ```
pub struct BenchmarkReportBuilder<'a> {
    metrics: &'a EngineMetrics,
    resources: ResourceMetrics,
    cpi: f64,
    tier: EfficiencyTier,
    elapsed_secs: f64,
    scenario_name: Option<String>,
}

impl<'a> BenchmarkReportBuilder<'a> {
    /// Create a new report builder.
    ///
    /// # Arguments
    ///
    /// - `metrics`: Engine counters (requests, success/fail, RPS, latency)
    /// - `resources`: CPU and memory snapshot at report time
    /// - `cpi`: Pre-calculated Cycles Per Instruction score
    /// - `tier`: Pre-classified efficiency tier from `EfficiencyTier::from_cpi(cpi)`
    /// - `elapsed_secs`: Total test duration in seconds (used to compute RPS)
    pub fn new(
        metrics: &'a EngineMetrics,
        resources: ResourceMetrics,
        cpi: f64,
        tier: EfficiencyTier,
        elapsed_secs: f64,
    ) -> Self {
        Self {
            metrics,
            resources,
            cpi,
            tier,
            elapsed_secs,
            scenario_name: None,
        }
    }

    /// Set the scenario name for the report header.
    pub fn scenario_name(mut self, name: &str) -> Self {
        self.scenario_name = Some(name.to_string());
        self
    }

    /// Generate the Markdown report string.
    ///
    /// The output contains two sections:
    /// 1. **Executive Summary** — overall pass/fail, scenario, duration, tier
    /// 2. **Engineer Metrics** — request counts, RPS, error rate, resource usage, CPI, P99
    pub fn build(&self) -> String {
        let total = self.metrics.total_requests();
        let success = self.metrics.successful_requests();
        let failed = self.metrics.failed_requests();
        let rps = self.metrics.rps(self.elapsed_secs);
        let error_rate = if total > 0 {
            (failed as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        let scenario = self.scenario_name.as_deref().unwrap_or("(unnamed)");

        // ── Executive verdict ────────────────────────────────────────────────
        let verdict = if error_rate < 1.0 && self.tier >= EfficiencyTier::Silver {
            "✅ PASS"
        } else if error_rate < 5.0 {
            "⚠️  MARGINAL"
        } else {
            "❌ FAIL"
        };

        // ── P99 latency (verification feature only) ──────────────────────────
        #[cfg(feature = "verification")]
        let (p50, p90, p99) = self.metrics.latency_percentiles();

        #[cfg(not(feature = "verification"))]
        let (p50, p90, p99): (u64, u64, u64) = (0, 0, 0);

        let latency_section = if cfg!(feature = "verification") {
            format!(
                "| P50 Latency       | {p50} ms          |\n\
                 | P90 Latency       | {p90} ms          |\n\
                 | P99 Latency       | {p99} ms          |\n"
            )
        } else {
            "| P99 Latency       | N/A (requires verification feature) |\n".to_string()
        };

        // ── Markdown assembly ────────────────────────────────────────────────
        format!(
            "# Laplace Benchmark Report\n\
             \n\
             ## Executive Summary\n\
             \n\
             | Field             | Value             |\n\
             |-------------------|-------------------|\n\
             | Verdict           | {verdict}         |\n\
             | Scenario          | {scenario}        |\n\
             | Duration          | {elapsed:.1}s     |\n\
             | Efficiency Tier   | {tier_emoji} {tier_name}  |\n\
             | CPI Score         | {cpi:.2}           |\n\
             \n\
             ## Engineer Metrics\n\
             \n\
             ### Throughput & Errors\n\
             \n\
             | Metric            | Value             |\n\
             |-------------------|-------------------|\n\
             | Total Requests    | {total}           |\n\
             | Successful        | {success}         |\n\
             | Failed            | {failed}          |\n\
             | Error Rate        | {error_rate:.2}%  |\n\
             | RPS               | {rps:.1} req/s    |\n\
             \n\
             ### Tail Latency\n\
             \n\
             | Metric            | Value             |\n\
             |-------------------|-------------------|\n\
             {latency_section}\
             \n\
             ### Resource Utilization\n\
             \n\
             | Metric            | Value             |\n\
             |-------------------|-------------------|\n\
             | CPU Usage         | {cpu:.1}%         |\n\
             | Memory Usage      | {mem:.1} MB       |\n\
             | CPI Score         | {cpi:.2}           |\n\
             ",
            verdict = verdict,
            scenario = scenario,
            elapsed = self.elapsed_secs,
            tier_emoji = self.tier.emoji(),
            tier_name = self.tier.name(),
            cpi = self.cpi,
            total = total,
            success = success,
            failed = failed,
            error_rate = error_rate,
            rps = rps,
            latency_section = latency_section,
            cpu = self.resources.cpu_percent,
            mem = self.resources.memory_mb,
        )
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_metrics(total: u64, success: u64, failed: u64) -> EngineMetrics {
        let m = EngineMetrics::new();
        for _ in 0..total {
            m.inc_requests();
        }
        for _ in 0..success {
            m.inc_successful();
        }
        for _ in 0..failed {
            m.inc_failed();
        }
        m
    }

    #[test]
    fn test_build_contains_header() {
        let m = make_metrics(100, 98, 2);
        let resources = ResourceMetrics::new(40.0, 512.0);
        let cpi = 6200.0;
        let tier = EfficiencyTier::from_cpi(cpi);
        let report = BenchmarkReportBuilder::new(&m, resources, cpi, tier, 10.0)
            .scenario_name("login_flow")
            .build();

        assert!(report.contains("# Laplace Benchmark Report"));
        assert!(report.contains("login_flow"));
        assert!(report.contains("## Executive Summary"));
        assert!(report.contains("## Engineer Metrics"));
    }

    #[test]
    fn test_build_contains_cpi_score() {
        let m = make_metrics(500, 490, 10);
        let resources = ResourceMetrics::new(55.0, 1024.0);
        let cpi = 8300.0;
        let tier = EfficiencyTier::from_cpi(cpi);
        let report = BenchmarkReportBuilder::new(&m, resources, cpi, tier, 30.0).build();

        assert!(report.contains("CPI Score"));
        assert!(report.contains("8300.00"));
    }

    #[test]
    fn test_build_contains_request_counts() {
        let m = make_metrics(1000, 950, 50);
        let resources = ResourceMetrics::new(70.0, 2048.0);
        let cpi = 2500.0;
        let tier = EfficiencyTier::from_cpi(cpi);
        let report = BenchmarkReportBuilder::new(&m, resources, cpi, tier, 60.0).build();

        assert!(report.contains("1000"));
        assert!(report.contains("950"));
        assert!(report.contains("50"));
        assert!(report.contains("Error Rate"));
        assert!(report.contains("RPS"));
    }

    #[test]
    fn test_build_verdict_pass() {
        // Low error rate + Silver or above → PASS
        let m = make_metrics(1000, 999, 1);
        let resources = ResourceMetrics::new(30.0, 256.0);
        let cpi = 11000.0; // Turbo
        let tier = EfficiencyTier::from_cpi(cpi);
        let report = BenchmarkReportBuilder::new(&m, resources, cpi, tier, 10.0).build();
        assert!(report.contains("PASS") || report.contains("✅"));
    }

    #[test]
    fn test_build_verdict_fail() {
        // High error rate → FAIL
        let m = make_metrics(100, 50, 50);
        let resources = ResourceMetrics::new(90.0, 4096.0);
        let cpi = 200.0; // Bronze
        let tier = EfficiencyTier::from_cpi(cpi);
        let report = BenchmarkReportBuilder::new(&m, resources, cpi, tier, 5.0).build();
        assert!(report.contains("FAIL") || report.contains("❌"));
    }

    #[test]
    fn test_build_resource_utilization() {
        let m = make_metrics(200, 195, 5);
        let resources = ResourceMetrics::new(42.5, 768.0);
        let cpi = 3000.0;
        let tier = EfficiencyTier::from_cpi(cpi);
        let report = BenchmarkReportBuilder::new(&m, resources, cpi, tier, 20.0).build();

        assert!(report.contains("42.5"));
        assert!(report.contains("768.0"));
        assert!(report.contains("CPU Usage"));
        assert!(report.contains("Memory Usage"));
    }
}
