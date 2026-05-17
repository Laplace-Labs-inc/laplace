// SPDX-License-Identifier: Apache-2.0
//! The Nerve: Stability Analysis Engine
//!
//! Analyzes tail latency (P50, P90, P99 percentiles) and event telemetry to assess
//! the stability of the system. Provides a stability score and tier classification.
//!
//! # Formula
//!
//! Stability Score = min(100.0, (P50 / P99) × 100.0)
//!
//! When P99 is 0 (no samples), the score defaults to 100.0 (optimal).

#[cfg(feature = "verification")]
use crate::domain::telemetry::TelemetryEvent;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// StabilityTier
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Classification of system stability based on the stability score.
///
/// # Tiers
///
/// - `RockSolid` (💎): Score ≥ 90.0 — Excellent stability
/// - `Stable` (✅): 75.0 ≤ Score < 90.0 — Good stability
/// - `Unstable` (⚠️): 50.0 ≤ Score < 75.0 — Acceptable but concerning
/// - `Critical` (🚨): Score < 50.0 — Poor stability, immediate attention required
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum StabilityTier {
    /// 💎 Rock Solid: score ≥ 90.0
    RockSolid,
    /// ✅ Stable: 75.0 ≤ score < 90.0
    Stable,
    /// ⚠️ Unstable: 50.0 ≤ score < 75.0
    Unstable,
    /// 🚨 Critical: score < 50.0
    Critical,
}

impl StabilityTier {
    /// Get the emoji icon for this tier
    pub fn icon(&self) -> &'static str {
        match self {
            StabilityTier::RockSolid => "💎",
            StabilityTier::Stable => "✅",
            StabilityTier::Unstable => "⚠️",
            StabilityTier::Critical => "🚨",
        }
    }

    /// Get the display name for this tier
    pub fn name(&self) -> &'static str {
        match self {
            StabilityTier::RockSolid => "RockSolid",
            StabilityTier::Stable => "Stable",
            StabilityTier::Unstable => "Unstable",
            StabilityTier::Critical => "Critical",
        }
    }

    /// Classify a stability score into a tier
    pub fn from_score(score: f64) -> Self {
        if score >= 90.0 {
            StabilityTier::RockSolid
        } else if score >= 75.0 {
            StabilityTier::Stable
        } else if score >= 50.0 {
            StabilityTier::Unstable
        } else {
            StabilityTier::Critical
        }
    }
}

impl std::fmt::Display for StabilityTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.icon(), self.name())
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// StabilityAnalyzer
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Analyzes latency percentiles to calculate a stability score.
///
/// Uses the formula: Stability Score = min(100.0, (P50 / P99) × 100.0)
///
/// When P99 is 0 (no samples), the score defaults to 100.0.
#[derive(Debug, Clone)]
pub struct StabilityAnalyzer;

impl StabilityAnalyzer {
    /// Calculate stability score from latency percentiles.
    ///
    /// # Arguments
    ///
    /// - `p50`: median latency in milliseconds
    /// - `p99`: 99th percentile latency in milliseconds
    ///
    /// # Returns
    ///
    /// A stability score between 0.0 and 100.0, where higher is better.
    /// - Returns 100.0 if p99 is 0 (no data)
    /// - Returns (p50 / p99) × 100.0, clamped to [0.0, 100.0]
    pub fn calculate_score(p50: u64, p99: u64) -> f64 {
        if p99 == 0 {
            return 100.0; // No samples yet, assume optimal
        }

        let score = (p50 as f64 / p99 as f64) * 100.0;
        score.clamp(0.0, 100.0) // Clamp to [0, 100]
    }

    /// Classify latency percentiles into a stability tier.
    pub fn classify(p50: u64, p99: u64) -> StabilityTier {
        let score = Self::calculate_score(p50, p99);
        StabilityTier::from_score(score)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// OutlierDetector
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Detects anomalies in telemetry events.
///
/// Counts error events (`TelemetryEvent::LogError` and `TelemetryEvent::ApiTrace` with `is_error: true`)
/// to provide anomaly metrics.
#[derive(Debug, Clone)]
pub struct OutlierDetector;

impl OutlierDetector {
    /// Count error events in a snapshot of telemetry events.
    ///
    /// Counts both `LogError` and `ApiTrace` events with `is_error: true`.
    #[cfg(feature = "verification")]
    pub fn count_errors(events: &[TelemetryEvent]) -> usize {
        events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    TelemetryEvent::LogError(_) | TelemetryEvent::ApiTrace { is_error: true, .. }
                )
            })
            .count()
    }

    /// Generate an anomaly detection report.
    #[cfg(feature = "verification")]
    pub fn analyze(events: &[TelemetryEvent]) -> String {
        let error_count = Self::count_errors(events);
        if error_count == 0 {
            "No anomalies detected".to_string()
        } else {
            format!("Detected {} recent anomalies", error_count)
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// NerveReport
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Comprehensive stability report combining latency analysis and anomaly detection.
///
/// # Fields
///
/// - `stability_score`: Calculated score [0, 100]
/// - `stability_tier`: Classification based on score
/// - `p50_ms`, `p90_ms`, `p99_ms`: Latency percentiles
/// - `anomaly_count`: Number of detected error events
/// - `anomaly_summary`: Human-readable anomaly report
#[derive(Debug, Clone)]
pub struct NerveReport {
    /// Stability score [0, 100]
    pub stability_score: f64,
    /// Tier classification
    pub stability_tier: StabilityTier,
    /// P50 latency in milliseconds
    pub p50_ms: u64,
    /// P90 latency in milliseconds
    pub p90_ms: u64,
    /// P99 latency in milliseconds
    pub p99_ms: u64,
    /// Count of detected anomalies
    pub anomaly_count: usize,
    /// Human-readable anomaly summary
    pub anomaly_summary: String,
}

impl NerveReport {
    /// Create a new nerve report from latency percentiles and events.
    #[cfg(feature = "verification")]
    pub fn new(p50_ms: u64, p90_ms: u64, p99_ms: u64, events: &[TelemetryEvent]) -> Self {
        let stability_score = StabilityAnalyzer::calculate_score(p50_ms, p99_ms);
        let stability_tier = StabilityTier::from_score(stability_score);
        let anomaly_count = OutlierDetector::count_errors(events);
        let anomaly_summary = OutlierDetector::analyze(events);

        Self {
            stability_score,
            stability_tier,
            p50_ms,
            p90_ms,
            p99_ms,
            anomaly_count,
            anomaly_summary,
        }
    }

    /// Format the report as a human-readable string.
    pub fn format(&self) -> String {
        format!(
            "{} Stability: {:.1} | Latency: P50={} P90={} P99={} ms | Anomalies: {}",
            self.stability_tier,
            self.stability_score,
            self.p50_ms,
            self.p90_ms,
            self.p99_ms,
            self.anomaly_count
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
    fn test_stability_tier_from_score() {
        assert_eq!(StabilityTier::from_score(95.0), StabilityTier::RockSolid);
        assert_eq!(StabilityTier::from_score(80.0), StabilityTier::Stable);
        assert_eq!(StabilityTier::from_score(60.0), StabilityTier::Unstable);
        assert_eq!(StabilityTier::from_score(40.0), StabilityTier::Critical);
    }

    #[test]
    fn test_stability_tier_icons() {
        assert_eq!(StabilityTier::RockSolid.icon(), "💎");
        assert_eq!(StabilityTier::Stable.icon(), "✅");
        assert_eq!(StabilityTier::Unstable.icon(), "⚠️");
        assert_eq!(StabilityTier::Critical.icon(), "🚨");
    }

    #[test]
    fn test_stability_analyzer_with_data() {
        // P50=50, P99=100 -> (50/100) * 100 = 50.0
        let score = StabilityAnalyzer::calculate_score(50, 100);
        assert_eq!(score, 50.0);

        // P50=75, P99=100 -> (75/100) * 100 = 75.0
        let score = StabilityAnalyzer::calculate_score(75, 100);
        assert_eq!(score, 75.0);

        // P50=100, P99=100 -> (100/100) * 100 = 100.0
        let score = StabilityAnalyzer::calculate_score(100, 100);
        assert_eq!(score, 100.0);
    }

    #[test]
    fn test_stability_analyzer_zero_p99() {
        // When P99 is 0, should return 100.0 (optimal)
        let score = StabilityAnalyzer::calculate_score(0, 0);
        assert_eq!(score, 100.0);

        let score = StabilityAnalyzer::calculate_score(50, 0);
        assert_eq!(score, 100.0);
    }

    #[test]
    fn test_stability_analyzer_classify() {
        assert_eq!(
            StabilityAnalyzer::classify(95, 100),
            StabilityTier::RockSolid
        );
        assert_eq!(StabilityAnalyzer::classify(80, 100), StabilityTier::Stable);
        assert_eq!(
            StabilityAnalyzer::classify(60, 100),
            StabilityTier::Unstable
        );
        assert_eq!(
            StabilityAnalyzer::classify(30, 100),
            StabilityTier::Critical
        );
    }

    #[cfg(feature = "verification")]
    #[test]
    fn test_outlier_detector_count_errors() {
        use crate::domain::entropy::seed::ContextId;

        let events = vec![
            TelemetryEvent::LogError("error 1".to_string()),
            TelemetryEvent::ApiTrace {
                method: "GET".to_string(),
                path: "/api/users".to_string(),
                payload: "{}".to_string(),
                is_error: true,
            },
            TelemetryEvent::StateChanged(ContextId::new(1), "Running".to_string()),
            TelemetryEvent::ApiTrace {
                method: "POST".to_string(),
                path: "/api/data".to_string(),
                payload: "{}".to_string(),
                is_error: false,
            },
            TelemetryEvent::LogError("error 2".to_string()),
        ];

        let count = OutlierDetector::count_errors(&events);
        assert_eq!(count, 3); // 2 LogErrors + 1 ApiTrace with is_error: true
    }

    #[cfg(feature = "verification")]
    #[test]
    fn test_outlier_detector_analyze() {
        use crate::domain::entropy::seed::ContextId;

        let events = vec![
            TelemetryEvent::LogError("error".to_string()),
            TelemetryEvent::StateChanged(ContextId::new(1), "Running".to_string()),
        ];

        let report = OutlierDetector::analyze(&events);
        assert_eq!(report, "Detected 1 recent anomalies");

        let empty_events: Vec<TelemetryEvent> = vec![];
        let report = OutlierDetector::analyze(&empty_events);
        assert_eq!(report, "No anomalies detected");
    }

    #[cfg(feature = "verification")]
    #[test]
    fn test_nerve_report_creation() {
        use crate::domain::entropy::seed::ContextId;

        let events = vec![
            TelemetryEvent::LogError("test error".to_string()),
            TelemetryEvent::StateChanged(ContextId::new(1), "Running".to_string()),
        ];

        let report = NerveReport::new(50, 75, 100, &events);

        assert_eq!(report.p50_ms, 50);
        assert_eq!(report.p90_ms, 75);
        assert_eq!(report.p99_ms, 100);
        assert_eq!(report.anomaly_count, 1);
        assert_eq!(report.stability_score, 50.0);
        assert_eq!(report.stability_tier, StabilityTier::Unstable);
    }

    #[cfg(feature = "verification")]
    #[test]
    fn test_nerve_report_format() {
        let events: Vec<TelemetryEvent> = vec![];
        let report = NerveReport::new(50, 75, 100, &events);

        let formatted = report.format();
        assert!(formatted.contains("Stability: 50.0"));
        assert!(formatted.contains("P50=50"));
        assert!(formatted.contains("P90=75"));
        assert!(formatted.contains("P99=100"));
        assert!(formatted.contains("Anomalies: 0"));
    }
}
