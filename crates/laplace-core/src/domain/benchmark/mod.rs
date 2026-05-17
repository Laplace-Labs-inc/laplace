// SPDX-License-Identifier: Apache-2.0
//! Benchmark & Resource Monitoring
//!
//! Provides resource monitoring, efficiency analysis, and stability assessment for the simulation engine.
//!
//! # Modules
//!
//! - [`nerve`]: Stability analysis and anomaly detection based on latency percentiles
//!   and event telemetry.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub mod cpi;
pub mod nerve;
pub mod report;
pub mod resource_monitor;
pub mod verdict;

#[cfg(kani)]
mod proofs;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Phase 4.2: ResourceMetrics & ResourceMonitor Trait
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Resource metrics captured at a point in time.
///
/// Contains CPU and memory utilization data for efficiency calculations.
pub struct ResourceMetrics {
    /// CPU usage percentage (0-100)
    pub cpu_percent: f64,
    /// Memory usage in megabytes
    pub memory_mb: f64,
}

impl ResourceMetrics {
    /// Create a new resource metrics snapshot.
    pub fn new(cpu_percent: f64, memory_mb: f64) -> Self {
        Self {
            cpu_percent,
            memory_mb,
        }
    }

    /// Check if metrics are within healthy bounds.
    pub fn is_healthy(&self) -> bool {
        self.cpu_percent <= 80.0 && self.memory_mb <= 8192.0
    }
}

/// Trait for types that can sample system resource usage.
///
/// Implemented by various resource monitoring backends (production, mock, simulation).
pub trait ResourceMonitor: Send + Sync {
    /// Sample current resource usage.
    fn sample(&self) -> ResourceMetrics;
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Phase 4.2: EfficiencyTier
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Classification of engine efficiency based on CPI (Cycles Per Instruction) metric.
///
/// Higher CPI indicates better resource efficiency (more work per unit resource consumed).
///
/// # Tiers
///
/// - `Bronze`: CPI < 1.0 — Poor efficiency
/// - `Silver`: 1.0 ≤ CPI < 2.0 — Acceptable efficiency
/// - `Gold`: 2.0 ≤ CPI < 3.5 — Good efficiency
/// - `Turbo`: CPI ≥ 3.5 — Excellent efficiency
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EfficiencyTier {
    /// Bronze: CPI < 1.0
    Bronze,
    /// Silver: 1.0 ≤ CPI < 2.0
    Silver,
    /// Gold: 2.0 ≤ CPI < 3.5
    Gold,
    /// Turbo: CPI ≥ 3.5
    Turbo,
}

impl EfficiencyTier {
    /// Classify a CPI value into an efficiency tier.
    ///
    /// # Arguments
    ///
    /// - `cpi`: Cycles Per Instruction metric (typically 0.0 to 5.0+)
    ///
    /// # Returns
    ///
    /// The corresponding efficiency tier.
    pub fn from_cpi(cpi: f64) -> Self {
        if cpi >= 10000.0 {
            EfficiencyTier::Turbo
        } else if cpi >= 5000.0 {
            EfficiencyTier::Gold
        } else if cpi >= 1000.0 {
            EfficiencyTier::Silver
        } else {
            EfficiencyTier::Bronze
        }
    }

    /// Get the emoji icon for this tier.
    pub fn emoji(&self) -> &'static str {
        match self {
            EfficiencyTier::Bronze => "🥉",
            EfficiencyTier::Silver => "🥈",
            EfficiencyTier::Gold => "🥇",
            EfficiencyTier::Turbo => "⚡",
        }
    }

    /// Get a human-readable description of this tier.
    pub fn description(&self) -> &'static str {
        match self {
            EfficiencyTier::Bronze => "Poor efficiency",
            EfficiencyTier::Silver => "Acceptable efficiency",
            EfficiencyTier::Gold => "Good efficiency",
            EfficiencyTier::Turbo => "Excellent efficiency",
        }
    }

    /// Get the tier name as a string.
    pub fn name(&self) -> &'static str {
        match self {
            EfficiencyTier::Bronze => "Bronze",
            EfficiencyTier::Silver => "Silver",
            EfficiencyTier::Gold => "Gold",
            EfficiencyTier::Turbo => "Turbo",
        }
    }
}

impl std::fmt::Display for EfficiencyTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Phase 4.2: CPICalculator
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Calculates Cycles Per Instruction (CPI) metric for engine efficiency analysis.
///
/// # Formula
///
/// CPI = (RPS × 1000) / (CPU_usage_percent + (Memory_usage_mb × 10))
///
/// Where:
/// - RPS: Requests Per Second processed by the engine
/// - CPU_usage_percent: CPU utilization (0-100)
/// - Memory_usage_mb: Memory consumption in megabytes
///
/// # Interpretation
///
/// - Higher CPI indicates better efficiency (more requests processed per resource unit)
/// - CPI < 1.0: Poor efficiency (high resource usage relative to throughput)
/// - CPI ≥ 3.5: Excellent efficiency (high throughput relative to resource usage)
pub struct CPICalculator {
    resource_monitor: Box<dyn ResourceMonitor>,
}

impl CPICalculator {
    /// Create a new CPI calculator with the given throughput and resource metrics.
    pub fn new(resource_monitor: Box<dyn ResourceMonitor>) -> Self {
        Self { resource_monitor }
    }

    /// Calculate the CPI (Cycles Per Instruction) value.
    ///
    /// # Returns
    ///
    /// The calculated CPI value. Returns 0.0 if denominator is 0.
    pub fn calculate_cpi(&self) -> f64 {
        use crate::domain::telemetry::GlobalTelemetry;

        // RPS 근사치 (테스트 및 데모용)
        let total_requests = GlobalTelemetry::metrics().total_requests() as f64;
        let rps = (total_requests / 10.0).max(1.0);

        let resources = self.resource_monitor.sample();
        let denominator = resources.cpu_percent + (resources.memory_mb * 10.0);

        if denominator < 0.1 {
            0.0
        } else {
            (rps * 1000.0) / denominator
        }
    }

    /// Evaluate the efficiency tier based on calculated CPI.
    pub fn evaluate(&self) -> (f64, EfficiencyTier) {
        let cpi = self.calculate_cpi();
        (cpi, EfficiencyTier::from_cpi(cpi))
    }

    /// Generate a human-readable efficiency report.
    pub fn report(&self) -> String {
        let (cpi, tier) = self.evaluate();
        let resources = self.resource_monitor.sample();

        format!(
            "CPI Benchmark Report\nCPU Usage: {:.1}%\nMemory: {:.1} MB\nCPI Score: {:.2}\nTier: {} {}",
            resources.cpu_percent, resources.memory_mb, cpi, tier.emoji(), tier
        )
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Phase 4.3: MockResourceMonitor
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Mock resource monitor for testing purposes.
///
/// Simulates resource consumption with scaled values for testing.
/// The scaling factor is 100x to enable fractional testing (e.g., 5.12 MB becomes 512).
#[derive(Debug, Clone)]
pub struct MockResourceMonitor {
    cpu_percent: Arc<AtomicU64>,
    memory_mb: Arc<AtomicU64>,
}

impl MockResourceMonitor {
    /// Create a new mock resource monitor with default values.
    ///
    /// # Defaults
    ///
    /// - `cpu_percent`: 50.0 (scaled: 5000)
    /// - `memory_mb`: 51200 (512.0 MB scaled by 100x)
    /// - `disk_io_mbps`: 100.0
    pub fn new() -> Self {
        Self {
            cpu_percent: Arc::new(AtomicU64::new(5000)), // 50.0 * 100.0
            memory_mb: Arc::new(AtomicU64::new(51200)),  // 512.0 * 100.0
        }
    }

    /// Update CPU usage
    pub fn set_cpu_percent(&self, percent: f64) {
        self.cpu_percent
            .store((percent * 100.0) as u64, Ordering::Relaxed);
    }

    /// Update memory usage (in scaled units)
    pub fn set_memory_mb(&self, mb: f64) {
        self.memory_mb.store((mb * 100.0) as u64, Ordering::Relaxed);
    }

    /// Update disk I/O throughput
    pub fn get_cpu_percent(&self) -> f64 {
        self.cpu_percent.load(Ordering::Relaxed) as f64 / 100.0
    }

    /// Get memory usage in scaled units
    pub fn get_memory_mb(&self) -> f64 {
        self.memory_mb.load(Ordering::Relaxed) as f64 / 100.0
    }
}

impl Default for MockResourceMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl ResourceMonitor for MockResourceMonitor {
    fn sample(&self) -> ResourceMetrics {
        ResourceMetrics::new(self.get_cpu_percent(), self.get_memory_mb())
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resource_metrics() {
        let metrics = ResourceMetrics::new(50.0, 2048.0);
        assert_eq!(metrics.cpu_percent, 50.0);
        assert_eq!(metrics.memory_mb, 2048.0);

        assert!(metrics.is_healthy());

        let unhealthy = ResourceMetrics::new(90.0, 9000.0);
        assert!(!unhealthy.is_healthy());
    }

    #[test]
    fn test_efficiency_tier() {
        // 새로운 임계값(1000, 5000, 10000) 테스트
        assert_eq!(EfficiencyTier::from_cpi(500.0), EfficiencyTier::Bronze);
        assert_eq!(EfficiencyTier::from_cpi(1500.0), EfficiencyTier::Silver);
        assert_eq!(EfficiencyTier::from_cpi(6000.0), EfficiencyTier::Gold);
        assert_eq!(EfficiencyTier::from_cpi(12000.0), EfficiencyTier::Turbo);
    }

    #[test]
    fn test_efficiency_tier_display() {
        assert_eq!(EfficiencyTier::Bronze.to_string(), "Bronze");
        assert_eq!(EfficiencyTier::Turbo.emoji(), "⚡");
    }

    #[test]
    fn test_mock_resource_monitor() {
        let monitor = MockResourceMonitor::new();
        // 초기값 확인 (50.0%, 512.0MB)
        assert_eq!(monitor.get_cpu_percent(), 50.0);
        assert_eq!(monitor.get_memory_mb(), 512.0);

        // Setter 테스트 (Atomic 변환 검증)
        monitor.set_cpu_percent(75.5);
        monitor.set_memory_mb(1024.0);

        assert_eq!(monitor.get_cpu_percent(), 75.5);
        assert_eq!(monitor.get_memory_mb(), 1024.0);
    }

    #[test]
    fn test_cpi_calculator() {
        let monitor = MockResourceMonitor::new();
        monitor.set_cpu_percent(10.0);
        monitor.set_memory_mb(100.0);

        let calc = CPICalculator::new(Box::new(monitor));
        let cpi = calc.calculate_cpi();

        // 0으로 나누기 방어가 잘 동작하고 양수가 나오는지 확인
        assert!(cpi > 0.0);

        let (score, _tier) = calc.evaluate();
        assert_eq!(score, cpi);

        // 리포트 텍스트 생성 검증
        let report = calc.report();
        assert!(report.contains("CPI Benchmark Report"));
        assert!(report.contains("CPU Usage: 10.0%"));
        assert!(report.contains("Memory: 100.0 MB"));
    }
}
