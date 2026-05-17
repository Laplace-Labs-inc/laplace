//! Extension traits for ResourceConfig and TenantMetadata (from laplace-interfaces)
//!
//! Migrated business logic for resource quotas, tenant identity, and tier management.

use crate::domain::now_ms;
use laplace_interfaces::domain::{ResourceConfig, TenantMetadata, TenantTier};
use laplace_interfaces::error::TenantError;
use std::time::Duration;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ResourceConfig Extension Trait
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Extension trait for ResourceConfig providing tier-based quota logic
pub trait ResourceConfigExt {
    /// Check if Turbo acceleration is enabled
    fn is_turbo_enabled(&self) -> bool;

    /// Check if Sentinel monitoring is enabled
    fn is_sentinel_enabled(&self) -> bool;
}

impl ResourceConfigExt for ResourceConfig {
    #[inline]
    fn is_turbo_enabled(&self) -> bool {
        self.use_turbo_acceleration
    }

    #[inline]
    fn is_sentinel_enabled(&self) -> bool {
        self.sentinel_monitoring
    }
}

/// Helper function to get resource configuration for a tier
///
/// # Arguments
///
/// * `tier` - Tenant tier to get configuration for
///
/// # Returns
///
/// Resource configuration matching the tier's capabilities
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "20_Core_Tenant",
        link = "LEP-0006-laplace-core-tenant_sovereignty"
    )
)]
pub fn resource_config_for_tier(tier: TenantTier) -> ResourceConfig {
    match tier {
        TenantTier::Free => ResourceConfig {
            max_concurrent_requests: 5,
            max_execution_time: Duration::from_millis(100),
            max_memory_mb: 128,
            max_cpu_time_ms: 100,
            use_turbo_acceleration: false,
            sentinel_monitoring: false,
        },

        TenantTier::Standard => ResourceConfig {
            max_concurrent_requests: 20,
            max_execution_time: Duration::from_millis(500),
            max_memory_mb: 512,
            max_cpu_time_ms: 500,
            use_turbo_acceleration: false,
            sentinel_monitoring: false,
        },

        TenantTier::Turbo => ResourceConfig {
            max_concurrent_requests: 100,
            max_execution_time: Duration::from_millis(2000),
            max_memory_mb: 2048,
            max_cpu_time_ms: 2000,
            use_turbo_acceleration: true,
            sentinel_monitoring: false,
        },

        TenantTier::Pro => ResourceConfig {
            max_concurrent_requests: 500,
            max_execution_time: Duration::from_millis(10_000),
            max_memory_mb: 8192,
            max_cpu_time_ms: 10_000,
            use_turbo_acceleration: true,
            sentinel_monitoring: false,
        },

        TenantTier::Enterprise => ResourceConfig {
            max_concurrent_requests: usize::MAX,
            max_execution_time: Duration::from_millis(60_000),
            max_memory_mb: u64::MAX,
            max_cpu_time_ms: u64::MAX,
            use_turbo_acceleration: true,
            sentinel_monitoring: true,
        },
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TenantMetadata Extension Trait
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Extension trait for TenantMetadata providing tenant management logic
pub trait TenantMetadataExt {
    /// Get current tier
    fn tier(&self) -> TenantTier;

    /// Get resource configuration for current tier
    fn resource_config(&self) -> ResourceConfig;

    /// Check if tenant uses Turbo acceleration
    fn uses_turbo(&self) -> bool;

    /// Check if tenant has Sentinel monitoring
    fn has_sentinel(&self) -> bool;

    /// Upgrade tenant to new tier
    ///
    /// # Business Rule
    ///
    /// Tier can only be upgraded, never downgraded.
    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(
            layer = "20_Core_Tenant",
            link = "LEP-0006-laplace-core-tenant_sovereignty"
        )
    )]
    fn upgrade_tier(&mut self, new_tier: TenantTier) -> Result<(), TenantError>;

    /// Validate tenant state
    ///
    /// # Business Rule
    ///
    /// Inactive tenants cannot execute code.
    fn validate(&self) -> Result<(), TenantError>;
}

impl TenantMetadataExt for TenantMetadata {
    #[inline]
    fn tier(&self) -> TenantTier {
        self.tier
    }

    #[inline]
    fn resource_config(&self) -> ResourceConfig {
        resource_config_for_tier(self.tier)
    }

    #[inline]
    fn uses_turbo(&self) -> bool {
        self.tier.uses_turbo_acceleration()
    }

    #[inline]
    fn has_sentinel(&self) -> bool {
        self.tier.has_sentinel_monitoring()
    }

    fn upgrade_tier(&mut self, new_tier: TenantTier) -> Result<(), TenantError> {
        if !self.tier.can_upgrade_to(new_tier) {
            return Err(TenantError::InvalidTierChange {
                current: self.tier,
                requested: new_tier,
            });
        }

        self.tier = new_tier;
        Ok(())
    }

    fn validate(&self) -> Result<(), TenantError> {
        if !self.active {
            return Err(TenantError::Inactive(self.tenant_id.clone()));
        }

        Ok(())
    }
}

/// Helper function to create new tenant metadata
///
/// # Arguments
///
/// * `tenant_id` - Unique tenant identifier
/// * `tier` - Initial subscription tier
///
/// # Returns
///
/// New tenant metadata with default values
pub fn create_tenant_metadata(tenant_id: String, tier: TenantTier) -> TenantMetadata {
    TenantMetadata {
        tenant_id: tenant_id.clone(),
        tier,
        resource_config: resource_config_for_tier(tier),
        active: true,
        created_at: now_ms(),
        storage_tree: format!("tenant_db_{}", tenant_id),
        fs_root: format!("root/tenants/{}", tenant_id),
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resource_config_for_tier() {
        let free_config = resource_config_for_tier(TenantTier::Free);
        assert!(!free_config.use_turbo_acceleration);
        assert!(!free_config.sentinel_monitoring);
        assert_eq!(free_config.max_concurrent_requests, 5);

        let turbo_config = resource_config_for_tier(TenantTier::Turbo);
        assert!(turbo_config.use_turbo_acceleration);
        assert!(!turbo_config.sentinel_monitoring);
        assert_eq!(turbo_config.max_concurrent_requests, 100);

        let enterprise_config = resource_config_for_tier(TenantTier::Enterprise);
        assert!(enterprise_config.use_turbo_acceleration);
        assert!(enterprise_config.sentinel_monitoring);
    }

    #[test]
    fn test_resource_config_ext() {
        let config = resource_config_for_tier(TenantTier::Turbo);
        assert!(config.is_turbo_enabled());

        let free_config = resource_config_for_tier(TenantTier::Free);
        assert!(!free_config.is_turbo_enabled());
        assert!(!free_config.is_sentinel_enabled());
    }

    #[test]
    fn test_create_tenant_metadata() {
        let tenant = create_tenant_metadata("test-123".to_string(), TenantTier::Free);

        assert_eq!(tenant.tenant_id, "test-123");
        assert_eq!(tenant.tier(), TenantTier::Free);
        assert_eq!(tenant.storage_tree, "tenant_db_test-123");
        assert_eq!(tenant.fs_root, "root/tenants/test-123");
        assert!(tenant.active);
    }

    #[test]
    fn test_tenant_tier_upgrade() {
        let mut tenant = create_tenant_metadata("test".to_string(), TenantTier::Free);

        // Valid upgrade
        assert!(tenant.upgrade_tier(TenantTier::Turbo).is_ok());
        assert_eq!(tenant.tier(), TenantTier::Turbo);
        assert!(tenant.uses_turbo());

        // Invalid downgrade
        let result = tenant.upgrade_tier(TenantTier::Standard);
        assert!(result.is_err());
        assert_eq!(tenant.tier(), TenantTier::Turbo);
    }

    #[test]
    fn test_tenant_validation() {
        let mut tenant = create_tenant_metadata("test".to_string(), TenantTier::Standard);

        // Active tenant is valid
        assert!(tenant.validate().is_ok());

        // Inactive tenant is invalid
        tenant.active = false;
        let result = tenant.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_tenant_resource_config_consistency() {
        let mut tenant = create_tenant_metadata("test".to_string(), TenantTier::Free);

        assert_eq!(tenant.resource_config().max_concurrent_requests, 5);
        assert!(!tenant.resource_config().use_turbo_acceleration);

        tenant.upgrade_tier(TenantTier::Turbo).unwrap();

        assert_eq!(tenant.resource_config().max_concurrent_requests, 100);
        assert!(tenant.resource_config().use_turbo_acceleration);
    }
}
