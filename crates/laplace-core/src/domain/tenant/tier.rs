// SPDX-License-Identifier: Apache-2.0
//! Extension trait for TenantTier (from laplace-interfaces)
//!
//! Migrated business logic for tier classification, capability checking, and progression.
//! Pure logic with no infrastructure dependencies.

use laplace_interfaces::domain::TenantTier;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Extension Trait
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Extension trait adding business logic to TenantTier
///
/// # Spec Compliance
///
/// - Sovereign-002: Tier determines resource allocation
/// - Sovereign-003: Tier-based concurrency and memory limits
/// - Performance: Turbo+ provides <500ns context sync vs ~41.5µs for Standard
pub trait TenantTierExt {
    /// Check if this tier qualifies for Turbo acceleration
    ///
    /// # Business Rule
    ///
    /// Only Turbo and above tiers get zero-copy shared memory optimization.
    /// This is a key differentiator in the pricing model.
    ///
    /// # Performance Impact
    ///
    /// - `false`: Context sync via Protobuf FFI (~41.5µs)
    /// - `true`: Context sync via shared memory (<500ns)
    fn uses_turbo_acceleration(self) -> bool;

    /// Check if this tier has Sentinel AI monitoring
    ///
    /// # Business Rule
    ///
    /// Only Enterprise tier gets advanced AI-powered anomaly detection.
    fn has_sentinel_monitoring(self) -> bool;

    /// Get human-readable tier name
    fn tier_name(self) -> &'static str;

    /// Check if upgrade to target tier is valid
    ///
    /// # Business Rule
    ///
    /// Tiers can only be upgraded (increased), never downgraded.
    /// Downgrades require manual intervention for billing reasons.
    fn can_upgrade_to(self, target: TenantTier) -> bool;

    /// Get next tier in progression
    fn next_tier(self) -> Option<TenantTier>;

    /// Get previous tier in progression
    fn previous_tier(self) -> Option<TenantTier>;
}

impl TenantTierExt for TenantTier {
    #[inline]
    fn uses_turbo_acceleration(self) -> bool {
        matches!(
            self,
            TenantTier::Turbo | TenantTier::Pro | TenantTier::Enterprise
        )
    }

    #[inline]
    fn has_sentinel_monitoring(self) -> bool {
        matches!(self, TenantTier::Enterprise)
    }

    #[inline]
    fn tier_name(self) -> &'static str {
        match self {
            TenantTier::Free => "Free",
            TenantTier::Standard => "Standard",
            TenantTier::Turbo => "Turbo",
            TenantTier::Pro => "Pro",
            TenantTier::Enterprise => "Enterprise",
        }
    }

    fn can_upgrade_to(self, target: TenantTier) -> bool {
        let current_value = self as u8;
        let target_value = target as u8;
        target_value > current_value
    }

    fn next_tier(self) -> Option<TenantTier> {
        match self {
            TenantTier::Free => Some(TenantTier::Standard),
            TenantTier::Standard => Some(TenantTier::Turbo),
            TenantTier::Turbo => Some(TenantTier::Pro),
            TenantTier::Pro => Some(TenantTier::Enterprise),
            TenantTier::Enterprise => None,
        }
    }

    fn previous_tier(self) -> Option<TenantTier> {
        match self {
            TenantTier::Free => None,
            TenantTier::Standard => Some(TenantTier::Free),
            TenantTier::Turbo => Some(TenantTier::Standard),
            TenantTier::Pro => Some(TenantTier::Turbo),
            TenantTier::Enterprise => Some(TenantTier::Pro),
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_turbo_acceleration_flag() {
        assert!(!TenantTier::Free.uses_turbo_acceleration());
        assert!(!TenantTier::Standard.uses_turbo_acceleration());
        assert!(TenantTier::Turbo.uses_turbo_acceleration());
        assert!(TenantTier::Pro.uses_turbo_acceleration());
        assert!(TenantTier::Enterprise.uses_turbo_acceleration());
    }

    #[test]
    fn test_sentinel_monitoring_flag() {
        assert!(!TenantTier::Free.has_sentinel_monitoring());
        assert!(!TenantTier::Standard.has_sentinel_monitoring());
        assert!(!TenantTier::Turbo.has_sentinel_monitoring());
        assert!(!TenantTier::Pro.has_sentinel_monitoring());
        assert!(TenantTier::Enterprise.has_sentinel_monitoring());
    }

    #[test]
    fn test_tier_names() {
        assert_eq!(TenantTier::Free.tier_name(), "Free");
        assert_eq!(TenantTier::Turbo.tier_name(), "Turbo");
        assert_eq!(TenantTier::Enterprise.tier_name(), "Enterprise");
    }

    #[test]
    fn test_upgrade_validation() {
        let free = TenantTier::Free;
        let standard = TenantTier::Standard;
        let turbo = TenantTier::Turbo;

        // Valid upgrades
        assert!(free.can_upgrade_to(standard));
        assert!(free.can_upgrade_to(turbo));
        assert!(standard.can_upgrade_to(TenantTier::Enterprise));

        // Invalid upgrades (downgrades)
        assert!(!turbo.can_upgrade_to(standard));
        assert!(!standard.can_upgrade_to(free));

        // Same tier (not an upgrade)
        assert!(!turbo.can_upgrade_to(turbo));
    }

    #[test]
    fn test_tier_progression() {
        assert_eq!(TenantTier::Free.next_tier(), Some(TenantTier::Standard));
        assert_eq!(TenantTier::Standard.next_tier(), Some(TenantTier::Turbo));
        assert_eq!(TenantTier::Turbo.next_tier(), Some(TenantTier::Pro));
        assert_eq!(TenantTier::Pro.next_tier(), Some(TenantTier::Enterprise));
        assert_eq!(TenantTier::Enterprise.next_tier(), None);

        assert_eq!(TenantTier::Free.previous_tier(), None);
        assert_eq!(TenantTier::Standard.previous_tier(), Some(TenantTier::Free));
        assert_eq!(
            TenantTier::Enterprise.previous_tier(),
            Some(TenantTier::Pro)
        );
    }
}
