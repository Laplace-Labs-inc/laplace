// SPDX-License-Identifier: Apache-2.0
//! Pool State and Health Monitoring
//!
//! Observability and diagnostics for resource pool utilization and health.
//! Provides metrics collection and health assessment independent of infrastructure.

// HealthStatus is now defined in laplace-interfaces
pub use laplace_interfaces::domain::pool::HealthStatus;

use serde::{Deserialize, Serialize};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Pool Snapshot: Metrics and Observability
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Complete snapshot of pool state for monitoring and observability
///
/// This structure captures the instantaneous state of resource pools, including
/// separate metrics for Turbo (zero-copy) and Standard (FFI) execution paths.
/// It enables comprehensive observability and capacity planning decisions.
///
/// # Architecture
///
/// The pool operates two parallel execution paths with independent capacity:
///
/// Turbo Path: Zero-copy shared memory acceleration available to Turbo, Pro, and
/// Enterprise tiers. Limited capacity requires careful allocation and preemption.
///
/// Standard Path: Protobuf FFI marshaling available to all tiers. Higher capacity
/// accommodates broader tenant base with acceptable latency characteristics.
///
/// # Spec Compliance
///
/// - Sovereign-001: Isolate pooling and lifecycle tracking
/// - Performance: Separate metrics for Turbo vs Standard paths
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolSnapshot {
    /// Total number of cached resources (warm pool size)
    ///
    /// This includes both Turbo and Standard path resources.
    pub cached_isolates: usize,

    /// Maximum total capacity across all execution paths
    pub max_capacity: usize,

    /// Overall pool health status
    pub healthy: bool,

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // Turbo Path Metrics (Zero-Copy Shared Memory)
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    /// Number of Turbo slots currently in active use
    ///
    /// Represents tenants with Turbo+ tier actively executing operations.
    pub turbo_active: usize,

    /// Total Turbo slot capacity in shared memory pool
    ///
    /// This capacity is fixed at initialization based on available shared memory.
    pub turbo_capacity: usize,

    /// Turbo pool utilization as percentage (0-100)
    ///
    /// Calculated as (turbo_active / turbo_capacity) * 100
    pub turbo_utilization_pct: u8,

    /// Cumulative count of Turbo pool exhaustion events
    ///
    /// Incremented when a Turbo-tier tenant cannot acquire a slot and must
    /// fall back to Standard FFI. High values indicate need for capacity expansion.
    pub turbo_fallback_count: u64,

    /// Average number of times each Turbo slot has been reused
    ///
    /// Higher values indicate efficient cache utilization and good temporal locality.
    pub turbo_avg_reuse_count: u64,

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // Standard Path Metrics (Protobuf FFI)
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    /// Number of Standard isolates currently in active use
    ///
    /// Includes both Free/Standard tier tenants and Turbo-tier fallbacks from
    /// pool exhaustion events.
    pub standard_active: usize,

    /// Total Standard isolate pool capacity
    ///
    /// This is the V8 isolate pool size for FFI-based execution.
    pub standard_capacity: usize,

    /// Standard pool utilization as percentage (0-100)
    ///
    /// Calculated as (standard_active / standard_capacity) * 100
    pub standard_utilization_pct: u8,

    /// Average lifetime of Standard isolates before eviction (seconds)
    ///
    /// Longer lifetimes indicate effective cache retention and warm pool efficiency.
    pub standard_avg_lifetime_secs: u64,
}

impl PoolSnapshot {
    /// Calculate overall pool utilization percentage
    ///
    /// # Returns
    ///
    /// Utilization percentage (0-100)
    pub fn overall_utilization_pct(&self) -> u8 {
        if self.max_capacity == 0 {
            return 0;
        }

        let used = self.cached_isolates;
        let total = self.max_capacity;

        ((used as f64 / total as f64) * 100.0) as u8
    }

    /// Check if pool is under pressure
    ///
    /// # Business Rule
    ///
    /// Pool is considered under pressure when overall utilization exceeds 80%,
    /// indicating potential capacity constraints.
    ///
    /// # Returns
    ///
    /// `true` if pool utilization is above threshold
    pub fn is_under_pressure(&self) -> bool {
        self.overall_utilization_pct() >= 80
    }

    /// Determine if Turbo pool should be scaled up
    ///
    /// # Business Rules
    ///
    /// Turbo pool requires scaling when either condition is met:
    /// - Utilization exceeds 90%, or
    /// - Fallback count exceeds 100, indicating frequent exhaustion
    ///
    /// # Returns
    ///
    /// `true` if Turbo pool expansion is recommended
    pub fn should_scale_turbo(&self) -> bool {
        self.turbo_utilization_pct > 90 || self.turbo_fallback_count > 100
    }

    /// Determine if Standard pool should be scaled up
    ///
    /// # Business Rule
    ///
    /// Standard pool requires scaling when utilization exceeds 85%.
    ///
    /// # Returns
    ///
    /// `true` if Standard pool expansion is recommended
    pub fn should_scale_standard(&self) -> bool {
        self.standard_utilization_pct > 85
    }

    /// Calculate Turbo adoption rate
    ///
    /// # Business Metric
    ///
    /// This metric tracks what percentage of active executions are utilizing
    /// the Turbo zero-copy path. Higher adoption rates indicate growing demand
    /// for performance-critical features and revenue opportunity through tier upgrades.
    ///
    /// # Returns
    ///
    /// Adoption rate as percentage (0.0-100.0)
    pub fn turbo_adoption_rate(&self) -> f64 {
        let total_active = self.turbo_active + self.standard_active;
        if total_active == 0 {
            return 0.0;
        }

        (self.turbo_active as f64 / total_active as f64) * 100.0
    }

    /// Calculate Turbo pool efficiency score
    ///
    /// # Business Metric
    ///
    /// Efficiency score combines reuse count and utilization to measure return on
    /// investment for Turbo infrastructure. The formula is:
    ///
    /// score = reuse_count * (utilization / 100)
    ///
    /// Higher scores indicate that Turbo slots are intensively reused at high
    /// utilization, maximizing infrastructure ROI.
    ///
    /// # Returns
    ///
    /// Efficiency score (0.0 to infinity)
    pub fn turbo_efficiency_score(&self) -> f64 {
        let utilization = self.turbo_utilization_pct as f64 / 100.0;
        self.turbo_avg_reuse_count as f64 * utilization
    }

    /// Get number of available Turbo slots
    ///
    /// # Returns
    ///
    /// Count of free Turbo slots available for allocation
    pub fn turbo_available(&self) -> usize {
        self.turbo_capacity.saturating_sub(self.turbo_active)
    }

    /// Get number of available Standard slots
    ///
    /// # Returns
    ///
    /// Count of free Standard isolates available for allocation
    pub fn standard_available(&self) -> usize {
        self.standard_capacity.saturating_sub(self.standard_active)
    }

    /// Check if Turbo pool has available capacity
    ///
    /// # Returns
    ///
    /// `true` if at least one Turbo slot is not in use
    pub fn has_turbo_capacity(&self) -> bool {
        self.turbo_available() > 0
    }

    /// Check if Standard pool has available capacity
    ///
    /// # Returns
    ///
    /// `true` if at least one Standard isolate is not in use
    pub fn has_standard_capacity(&self) -> bool {
        self.standard_available() > 0
    }
}

impl Default for PoolSnapshot {
    fn default() -> Self {
        Self {
            cached_isolates: 0,
            max_capacity: 0,
            healthy: true,
            turbo_active: 0,
            turbo_capacity: 0,
            turbo_utilization_pct: 0,
            turbo_fallback_count: 0,
            turbo_avg_reuse_count: 0,
            standard_active: 0,
            standard_capacity: 0,
            standard_utilization_pct: 0,
            standard_avg_lifetime_secs: 0,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Health Check: Pure Assessment Logic
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Pool health assessment algorithm
///
/// This structure encapsulates pure decision logic for classifying pool health
/// based on metrics. The assessment uses tiered thresholds to provide actionable
/// guidance for monitoring systems and automation.
pub struct PoolHealthCheck;

impl PoolHealthCheck {
    /// Assess pool health based on current metrics
    ///
    /// # Assessment Logic
    ///
    /// Health classification follows a priority order:
    ///
    /// 1. **Unhealthy** if:
    ///    - Overall utilization ≥ 95% (near capacity), OR
    ///    - Both Turbo and Standard pools exceed 95% (complete exhaustion)
    ///
    /// 2. **Degraded** if:
    ///    - Turbo utilization > 85% (under pressure), OR
    ///    - Turbo fallback count > 50 (frequent exhaustion), OR
    ///    - Standard utilization > 85% (under pressure)
    ///
    /// 3. **Healthy** otherwise
    ///
    /// # Arguments
    ///
    /// * `snapshot` - Current pool state metrics
    ///
    /// # Returns
    ///
    /// Health status with descriptive reason if not healthy
    pub fn assess(snapshot: &PoolSnapshot) -> HealthStatus {
        // Critical: Overall pool near capacity
        if snapshot.overall_utilization_pct() >= 95 {
            return HealthStatus::Unhealthy {
                reason: "Pool near capacity".to_string(),
            };
        }

        // Critical: Both execution paths exhausted
        if snapshot.turbo_utilization_pct > 95 && snapshot.standard_utilization_pct > 95 {
            return HealthStatus::Unhealthy {
                reason: "All pools exhausted".to_string(),
            };
        }

        // Warning: Turbo path under pressure
        if snapshot.turbo_utilization_pct > 85 {
            return HealthStatus::Degraded {
                reason: "Turbo pool under pressure".to_string(),
            };
        }

        // Warning: Frequent Turbo exhaustion
        if snapshot.turbo_fallback_count > 50 {
            return HealthStatus::Degraded {
                reason: "High Turbo fallback rate".to_string(),
            };
        }

        // Warning: Standard path under pressure
        if snapshot.standard_utilization_pct > 85 {
            return HealthStatus::Degraded {
                reason: "Standard pool under pressure".to_string(),
            };
        }

        HealthStatus::Healthy
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_snapshot_utilization() {
        let snapshot = PoolSnapshot {
            cached_isolates: 85,
            max_capacity: 100,
            turbo_active: 30,
            turbo_capacity: 50,
            standard_active: 10,
            standard_capacity: 50,
            turbo_utilization_pct: 60,
            standard_utilization_pct: 20,
            ..Default::default()
        };

        assert_eq!(snapshot.overall_utilization_pct(), 85);
        assert!(snapshot.is_under_pressure());
    }

    #[test]
    fn test_turbo_adoption_rate() {
        let snapshot = PoolSnapshot {
            turbo_active: 30,
            standard_active: 10,
            ..Default::default()
        };

        assert_eq!(snapshot.turbo_adoption_rate(), 75.0);
    }

    #[test]
    fn test_turbo_efficiency_score() {
        let snapshot = PoolSnapshot {
            turbo_utilization_pct: 80,
            turbo_avg_reuse_count: 100,
            ..Default::default()
        };

        assert_eq!(snapshot.turbo_efficiency_score(), 80.0);
    }

    #[test]
    fn test_available_slots() {
        let snapshot = PoolSnapshot {
            turbo_active: 30,
            turbo_capacity: 50,
            standard_active: 40,
            standard_capacity: 50,
            ..Default::default()
        };

        assert_eq!(snapshot.turbo_available(), 20);
        assert_eq!(snapshot.standard_available(), 10);
        assert!(snapshot.has_turbo_capacity());
        assert!(snapshot.has_standard_capacity());
    }

    #[test]
    fn test_scaling_recommendations() {
        let snapshot1 = PoolSnapshot {
            turbo_utilization_pct: 95,
            turbo_fallback_count: 150,
            ..Default::default()
        };
        assert!(snapshot1.should_scale_turbo());

        let snapshot2 = PoolSnapshot {
            standard_utilization_pct: 90,
            ..Default::default()
        };
        assert!(snapshot2.should_scale_standard());

        let snapshot3 = PoolSnapshot {
            turbo_utilization_pct: 50,
            standard_utilization_pct: 50,
            turbo_fallback_count: 10,
            ..Default::default()
        };
        assert!(!snapshot3.should_scale_turbo());
        assert!(!snapshot3.should_scale_standard());
    }

    #[test]
    fn test_health_assessment() {
        let healthy = PoolSnapshot {
            cached_isolates: 50,
            max_capacity: 100,
            turbo_utilization_pct: 50,
            standard_utilization_pct: 50,
            turbo_fallback_count: 10,
            ..Default::default()
        };
        assert_eq!(PoolHealthCheck::assess(&healthy), HealthStatus::Healthy);

        let degraded1 = PoolSnapshot {
            turbo_utilization_pct: 90,
            ..Default::default()
        };
        assert!(matches!(
            PoolHealthCheck::assess(&degraded1),
            HealthStatus::Degraded { .. }
        ));

        let degraded2 = PoolSnapshot {
            turbo_fallback_count: 60,
            ..Default::default()
        };
        assert!(matches!(
            PoolHealthCheck::assess(&degraded2),
            HealthStatus::Degraded { .. }
        ));

        let unhealthy = PoolSnapshot {
            cached_isolates: 96,
            max_capacity: 100,
            ..Default::default()
        };
        assert!(matches!(
            PoolHealthCheck::assess(&unhealthy),
            HealthStatus::Unhealthy { .. }
        ));
    }

    #[test]
    fn test_health_status_checks() {
        let healthy = HealthStatus::Healthy;
        assert!(healthy.is_healthy());
        assert!(!healthy.is_degraded());
        assert!(!healthy.is_unhealthy());
        assert_eq!(healthy.reason(), None);

        let degraded = HealthStatus::Degraded {
            reason: "Test reason".to_string(),
        };
        assert!(!degraded.is_healthy());
        assert!(degraded.is_degraded());
        assert_eq!(degraded.reason(), Some("Test reason"));

        let unhealthy = HealthStatus::Unhealthy {
            reason: "Critical".to_string(),
        };
        assert!(unhealthy.is_unhealthy());
        assert_eq!(unhealthy.reason(), Some("Critical"));
    }

    #[test]
    fn test_serialization() {
        let snapshot = PoolSnapshot {
            cached_isolates: 50,
            max_capacity: 100,
            turbo_active: 20,
            turbo_capacity: 30,
            turbo_utilization_pct: 66,
            standard_active: 30,
            standard_capacity: 70,
            ..Default::default()
        };

        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains("cached_isolates"));
        assert!(json.contains("turbo_active"));

        let deserialized: PoolSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.cached_isolates, 50);
        assert_eq!(deserialized.turbo_active, 20);
    }
}
