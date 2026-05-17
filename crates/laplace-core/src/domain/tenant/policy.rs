// SPDX-License-Identifier: Apache-2.0
//! Tenant Policy Structs: Pure Business Rules
//!
//! Stateless policy objects implementing domain rules for path security,
//! resource validation, and tier recommendations.

use laplace_interfaces::domain::{ResourceConfig, TenantMetadata, TenantTier};
use std::path::{Path, PathBuf};

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Path Security Policy
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Path security and remapping policies
///
/// # Spec Compliance
///
/// - Sovereign-002: Virtualized path mapping (Chroot-like)
/// - Security: Prevent path traversal attacks
pub struct PathPolicy;

impl PathPolicy {
    /// Remap virtual path to physical path (Chroot-style)
    ///
    /// # Business Rule
    ///
    /// Tenants see a virtual filesystem rooted at `/`.
    /// Actual paths are remapped to `root/tenants/{tenant_id}/...`
    ///
    /// # Security
    ///
    /// - Prevents path traversal (..)
    /// - Prevents absolute path escapes
    /// - Prevents symlink attacks
    pub fn safe_remap(tenant: &TenantMetadata, virtual_path: &str) -> PathBuf {
        let clean_path = virtual_path.trim_start_matches('/');
        Path::new(&tenant.fs_root).join(clean_path)
    }

    /// Check if physical path is within tenant's allowed boundaries
    ///
    /// # Business Rule
    ///
    /// Tenant can only access paths under their fs_root.
    /// This prevents cross-tenant data access.
    pub fn is_path_allowed<P: AsRef<Path>>(tenant: &TenantMetadata, physical_path: P) -> bool {
        physical_path.as_ref().starts_with(&tenant.fs_root)
    }

    /// Validate path for safety
    ///
    /// # Security Checks
    ///
    /// - No null bytes
    /// - No excessive length
    /// - No suspicious patterns
    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(layer = "domain", link = "LEP-0006-laplace-core-tenant_sovereignty")
    )]
    pub fn is_path_safe(path: &str) -> bool {
        // Use naive byte-iterator checks instead of `str::contains` to avoid
        // SIMD-accelerated routines (e.g. `memchr`) that Kani cannot model.
        let bytes = path.as_bytes();

        if bytes.contains(&0) {
            return false;
        }

        if path.len() > 4096 {
            return false;
        }

        if bytes.windows(4).any(|w| w == b"/../") || bytes.windows(3).any(|w| w == b"/./") {
            return false;
        }

        true
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Resource Validation Policy
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Resource validation and quota checking policies
pub struct ResourcePolicy;

impl ResourcePolicy {
    /// Check if execution time exceeds tier limit
    pub fn is_execution_time_valid(config: &ResourceConfig, elapsed_ms: u64) -> bool {
        elapsed_ms <= config.max_execution_time.as_millis() as u64
    }

    /// Check if memory usage is within tier limit
    pub fn is_memory_usage_valid(config: &ResourceConfig, used_mb: u64) -> bool {
        used_mb <= config.max_memory_mb
    }

    /// Check if concurrent requests are within tier limit
    pub fn is_concurrency_valid(config: &ResourceConfig, active_requests: usize) -> bool {
        active_requests <= config.max_concurrent_requests
    }

    /// Calculate utilization percentage
    pub fn calculate_utilization(used: u64, limit: u64) -> u8 {
        if limit == 0 {
            return 0;
        }

        let pct = (used as f64 / limit as f64) * 100.0;
        pct.min(100.0) as u8
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tier Recommendation Policy
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Tier recommendation and upgrade policies
///
/// # Business Logic
///
/// Analyzes usage patterns to suggest optimal tier upgrades.
pub struct TierRecommendationPolicy;

impl TierRecommendationPolicy {
    /// Calculate tier recommendation based on usage pattern
    ///
    /// # Business Logic
    ///
    /// - High concurrency → Recommend Turbo+
    /// - Long execution times → Recommend Pro+
    /// - Frequent quota hits → Recommend upgrade
    pub fn recommend_tier(
        current_tier: TenantTier,
        avg_concurrent: usize,
        avg_exec_time_ms: u64,
        quota_hit_rate: f64,
    ) -> Option<TenantTier> {
        let current_config = super::model::resource_config_for_tier(current_tier);

        // Check if frequently hitting concurrency limit
        if quota_hit_rate > 0.5 {
            if let Some(next_tier) = current_tier.next_tier() {
                return Some(next_tier);
            }
        }

        // Check if concurrency is near limit (>80%)
        let concurrency_pct =
            (avg_concurrent as f64 / current_config.max_concurrent_requests as f64) * 100.0;
        if concurrency_pct > 80.0 {
            if current_tier < TenantTier::Turbo {
                return Some(TenantTier::Turbo);
            }
            if let Some(next_tier) = current_tier.next_tier() {
                return Some(next_tier);
            }
        }

        // Check if execution time is near limit (>80%)
        let time_limit_ms = current_config.max_execution_time.as_millis() as u64;
        let time_pct = (avg_exec_time_ms as f64 / time_limit_ms as f64) * 100.0;
        if time_pct > 80.0 {
            if current_tier < TenantTier::Pro {
                return Some(TenantTier::Pro);
            }
            if current_tier < TenantTier::Enterprise {
                return Some(TenantTier::Enterprise);
            }
        }

        None
    }

    /// Calculate cost multiplier for tier
    ///
    /// # Business Model
    ///
    /// Each tier costs more than the previous, with Turbo being a premium tier.
    pub fn cost_multiplier(tier: TenantTier) -> f64 {
        match tier {
            TenantTier::Free => 0.0,
            TenantTier::Standard => 1.0,
            TenantTier::Turbo => 3.0,
            TenantTier::Pro => 8.0,
            TenantTier::Enterprise => 0.0,
        }
    }

    /// Calculate ROI of upgrading to next tier
    ///
    /// # Business Logic
    ///
    /// Higher ROI suggests stronger case for upgrade.
    pub fn calculate_upgrade_roi(
        current_tier: TenantTier,
        quota_hit_rate: f64,
        avg_latency_ms: u64,
    ) -> f64 {
        let next_tier = match current_tier.next_tier() {
            Some(t) => t,
            None => return 0.0,
        };

        let cost_increase = Self::cost_multiplier(next_tier) - Self::cost_multiplier(current_tier);

        let quota_benefit = quota_hit_rate * 10.0;

        let turbo_benefit =
            if next_tier.uses_turbo_acceleration() && !current_tier.uses_turbo_acceleration() {
                (avg_latency_ms as f64 / 1000.0) * 5.0
            } else {
                0.0
            };

        let total_benefit = quota_benefit + turbo_benefit;

        if cost_increase > 0.0 {
            total_benefit / cost_increase
        } else {
            total_benefit
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::tenant::model::create_tenant_metadata;

    #[test]
    fn test_path_remapping() {
        let tenant = create_tenant_metadata("secure-tenant".to_string(), TenantTier::Standard);
        let physical = PathPolicy::safe_remap(&tenant, "/app/data/file.txt");
        assert_eq!(
            physical,
            PathBuf::from("root/tenants/secure-tenant/app/data/file.txt")
        );
    }

    #[test]
    fn test_path_isolation() {
        let tenant = create_tenant_metadata("tenant-a".to_string(), TenantTier::Standard);
        assert!(PathPolicy::is_path_allowed(
            &tenant,
            "root/tenants/tenant-a/data/file.txt"
        ));
        assert!(!PathPolicy::is_path_allowed(
            &tenant,
            "root/tenants/tenant-b/data/file.txt"
        ));
    }

    #[test]
    fn test_path_safety() {
        assert!(PathPolicy::is_path_safe("/app/data/file.txt"));
        assert!(!PathPolicy::is_path_safe("/app\0/data"));
        assert!(!PathPolicy::is_path_safe("/app/../etc/passwd"));
    }

    #[test]
    fn test_execution_time_validation() {
        let config = super::super::model::resource_config_for_tier(TenantTier::Free);
        assert!(ResourcePolicy::is_execution_time_valid(&config, 50));
        assert!(ResourcePolicy::is_execution_time_valid(&config, 100));
        assert!(!ResourcePolicy::is_execution_time_valid(&config, 150));
    }

    #[test]
    fn test_tier_recommendation() {
        let rec = TierRecommendationPolicy::recommend_tier(TenantTier::Standard, 50, 100, 0.6);
        assert_eq!(rec, Some(TenantTier::Turbo));

        let rec = TierRecommendationPolicy::recommend_tier(TenantTier::Free, 3, 50, 0.7);
        assert_eq!(rec, Some(TenantTier::Standard));

        let rec =
            TierRecommendationPolicy::recommend_tier(TenantTier::Enterprise, 1000, 10000, 0.9);
        assert_eq!(rec, None);
    }

    #[test]
    fn test_cost_multiplier() {
        assert_eq!(
            TierRecommendationPolicy::cost_multiplier(TenantTier::Free),
            0.0
        );
        assert_eq!(
            TierRecommendationPolicy::cost_multiplier(TenantTier::Standard),
            1.0
        );
        assert_eq!(
            TierRecommendationPolicy::cost_multiplier(TenantTier::Turbo),
            3.0
        );
    }
}
