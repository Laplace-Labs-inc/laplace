//! # Domain Model: Tenant Subscription Tier Classification
//!
//! Pure business logic for tenant subscription tiers and their capabilities.
//! This module is the single source of truth for tier hierarchy, feature access,
//! and resource allocation boundaries.
//!
//! ## Tier Architecture
//!
//! The Laplace platform defines a five-tier subscription model that gates access
//! to performance optimizations, resource limits, and monitoring features:
//!
//! ```text
//! Free → Standard → Turbo → Pro → Enterprise
//! ↓ ↓ ↓ ↓ ↓
//! Eval Basic Optimized Premium Custom SLA
//! ```
//!
//! ## Key Differentiators
//!
//! **Turbo Acceleration**: Only Turbo and above tiers receive zero-copy shared
//! memory FFI optimization. This boundary is enforced by `uses_turbo_acceleration()`.
//! Performance difference: ~41.5µs (Standard FFI) vs <500ns (Turbo+ shared memory).
//!
//! **Sentinel Monitoring**: Only Enterprise tier gets advanced AI-powered anomaly
//! detection for production workloads.

use serde::{Deserialize, Serialize};
use std::fmt;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Tenant subscription tier.
///
/// Classifies tenants into service levels that determine performance profiles,
/// resource limits, and feature availability. Maps directly to billing and SLA
/// boundaries, with each tier unlocking specific optimizations and guarantees.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Tenant",
        link = "LEP-0007-laplace-interfaces-tenant_model"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum TenantTier {
    /// Free tier (0) - Evaluation and hobbyist use
    ///
    /// Entry-level tier suitable for evaluation, prototyping, and hobbyist projects.
    ///
    /// **Characteristics:**
    /// - Standard FFI execution only (Protobuf serialization)
    /// - 5 concurrent requests maximum
    /// - 100ms execution timeout
    /// - 128MB memory limit
    /// - No turbo acceleration
    /// - Community support only
    Free = 0,

    /// Standard tier (1) - Small business and professional use
    ///
    /// Baseline tier designed for small teams, freelancers, and professional
    /// developers building production applications with moderate scale.
    ///
    /// **Characteristics:**
    /// - Standard FFI execution (Protobuf serialization)
    /// - 20 concurrent requests
    /// - 500ms execution timeout
    /// - 512MB memory limit
    /// - Default tier for new paid accounts
    /// - Email support
    Standard = 1,

    /// Turbo tier (2) - Performance-focused customers
    ///
    /// High-performance tier unlocking zero-copy shared memory optimization for
    /// microsecond-latency request handling. Designed for latency-sensitive
    /// applications and high-frequency workloads.
    ///
    /// **Characteristics:**
    /// - **⚡ Zero-copy shared memory acceleration** (<500ns context sync)
    /// - 100 concurrent requests
    /// - 2 second execution timeout
    /// - 2GB memory limit
    /// - Significant performance improvement over Standard FFI
    /// - Priority scheduling within tenant workloads
    /// - Priority support (24hr response SLA)
    Turbo = 2,

    /// Pro tier (3) - Power users and growing teams
    ///
    /// Enterprise-ready tier for power users, growing teams, and applications
    /// with demanding performance requirements. Includes advanced monitoring
    /// and resource guarantees.
    ///
    /// **Characteristics:**
    /// - **⚡ Zero-copy acceleration** with reserved memory pools
    /// - 500 concurrent requests
    /// - 10 second execution timeout
    /// - 8GB memory limit
    /// - Advanced feature flags and beta access
    /// - Dedicated support (2hr response SLA)
    /// - Custom domain support
    Pro = 3,

    /// Enterprise tier (4) - Mission-critical deployments
    ///
    /// Maximum tier reserved for mission-critical deployments with custom SLA
    /// agreements. Includes advanced monitoring, guaranteed resource isolation,
    /// and Sentinel AI anomaly detection.
    ///
    /// **Characteristics:**
    /// - **⚡ Zero-copy acceleration** with guaranteed isolation
    /// - Unlimited concurrency (with soft limits for multi-tenancy fairness)
    /// - 60 second execution timeout
    /// - Unlimited memory (with soft limits)
    /// - **🛡️ Sentinel AI monitoring** for anomaly detection and security
    /// - Custom SLA with uptime guarantees
    /// - Dedicated infrastructure and support
    /// - Technical account manager
    Enterprise = 4,
}

impl TenantTier {
    /// Check if this tier qualifies for zero-copy Turbo acceleration.
    ///
    /// # Business Rule
    ///
    /// Only Turbo tier and above (Turbo, Pro, Enterprise) have access to
    /// shared memory zero-copy optimization. This is a key pricing differentiator.
    ///
    /// # Performance Impact
    ///
    /// - `false`: Context sync via Protobuf FFI (~41.5µs per sync)
    /// - `true`: Context sync via shared memory (<500ns per sync)
    ///
    /// # Returns
    ///
    /// `true` if this tier has Turbo acceleration enabled.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// assert!(!TenantTier::Free.uses_turbo_acceleration());
    /// assert!(TenantTier::Turbo.uses_turbo_acceleration());
    /// assert!(TenantTier::Enterprise.uses_turbo_acceleration());
    /// ```
    #[inline]
    pub const fn uses_turbo_acceleration(self) -> bool {
        matches!(self, Self::Turbo | Self::Pro | Self::Enterprise)
    }

    /// Check if this tier has Sentinel AI monitoring enabled.
    ///
    /// # Business Rule
    ///
    /// Only Enterprise tier receives advanced AI-powered anomaly detection,
    /// intrusion detection, and security monitoring.
    ///
    /// # Returns
    ///
    /// `true` if Sentinel monitoring is available.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// assert!(!TenantTier::Pro.has_sentinel_monitoring());
    /// assert!(TenantTier::Enterprise.has_sentinel_monitoring());
    /// ```
    #[inline]
    pub const fn has_sentinel_monitoring(self) -> bool {
        matches!(self, Self::Enterprise)
    }

    /// Get human-readable tier name.
    ///
    /// # Returns
    ///
    /// Static string slice with tier name suitable for display and logging.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// assert_eq!(TenantTier::Turbo.name(), "Turbo");
    /// ```
    #[inline]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Free => "Free",
            Self::Standard => "Standard",
            Self::Turbo => "Turbo",
            Self::Pro => "Pro",
            Self::Enterprise => "Enterprise",
        }
    }

    /// Parse tier from numeric value with validation.
    ///
    /// # Arguments
    ///
    /// * `value` - Numeric tier value in range [0, 4]
    ///
    /// # Returns
    ///
    /// `Some(TenantTier)` if valid value, `None` if out of range.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// assert_eq!(TenantTier::from_u8(2), Some(TenantTier::Turbo));
    /// assert_eq!(TenantTier::from_u8(5), None);
    /// ```
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Free),
            1 => Some(Self::Standard),
            2 => Some(Self::Turbo),
            3 => Some(Self::Pro),
            4 => Some(Self::Enterprise),
            _ => None,
        }
    }

    /// Get numeric tier value.
    ///
    /// # Returns
    ///
    /// Tier as u8 in range [0, 4].
    ///
    /// # Examples
    ///
    /// ```ignore
    /// assert_eq!(TenantTier::Enterprise.as_u8(), 4);
    /// ```
    #[inline]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Check if an upgrade to target tier is valid.
    ///
    /// # Business Rule
    ///
    /// Tiers can only be upgraded (increased ordinal), never downgraded.
    /// Downgrades require manual intervention through billing systems for
    /// compliance and audit trail requirements.
    ///
    /// # Arguments
    ///
    /// * `target` - Target tier to upgrade to
    ///
    /// # Returns
    ///
    /// `true` if upgrade is valid (target tier > current tier), `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// assert!(TenantTier::Standard.can_upgrade_to(TenantTier::Turbo));
    /// assert!(!TenantTier::Turbo.can_upgrade_to(TenantTier::Standard));
    /// assert!(!TenantTier::Turbo.can_upgrade_to(TenantTier::Turbo));
    /// ```
    pub fn can_upgrade_to(self, target: Self) -> bool {
        target.as_u8() > self.as_u8()
    }

    /// Get the next tier in progression.
    ///
    /// Returns the immediately higher tier in the subscription hierarchy.
    /// Useful for UI flow, tier recommendations, and upgrade prompts.
    ///
    /// # Returns
    ///
    /// `Some(TenantTier)` for the next tier, `None` if already at Enterprise.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// assert_eq!(TenantTier::Standard.next_tier(), Some(TenantTier::Turbo));
    /// assert_eq!(TenantTier::Enterprise.next_tier(), None);
    /// ```
    pub fn next_tier(self) -> Option<Self> {
        Self::from_u8(self.as_u8() + 1)
    }

    /// Get the previous tier (for reference only).
    ///
    /// Returns the immediately lower tier in the subscription hierarchy.
    /// Used for context only; downgrades require manual intervention.
    ///
    /// # Returns
    ///
    /// `Some(TenantTier)` for the previous tier, `None` if already at Free.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// assert_eq!(TenantTier::Turbo.previous_tier(), Some(TenantTier::Standard));
    /// assert_eq!(TenantTier::Free.previous_tier(), None);
    /// ```
    pub fn previous_tier(self) -> Option<Self> {
        if self.as_u8() == 0 {
            None
        } else {
            Self::from_u8(self.as_u8() - 1)
        }
    }

    /// Check if this tier supports turbo-mode acceleration.
    ///
    /// # Returns
    ///
    /// `true` if the tier permits zero-copy shared memory FFI, `false` otherwise.
    ///
    /// Turbo-mode is available for Turbo, Pro, and Enterprise tiers.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// assert!(!TenantTier::Free.supports_turbo());
    /// assert!(TenantTier::Turbo.supports_turbo());
    /// assert!(TenantTier::Enterprise.supports_turbo());
    /// ```
    #[inline]
    pub fn supports_turbo(&self) -> bool {
        matches!(
            self,
            TenantTier::Turbo | TenantTier::Pro | TenantTier::Enterprise
        )
    }
}

#[allow(clippy::derivable_impls)]
impl Default for TenantTier {
    fn default() -> Self {
        Self::Free
    }
}

impl fmt::Display for TenantTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_hierarchy() {
        assert_eq!(TenantTier::Free.as_u8(), 0);
        assert_eq!(TenantTier::Standard.as_u8(), 1);
        assert_eq!(TenantTier::Turbo.as_u8(), 2);
        assert_eq!(TenantTier::Pro.as_u8(), 3);
        assert_eq!(TenantTier::Enterprise.as_u8(), 4);
    }

    #[test]
    fn turbo_acceleration_feature_flag() {
        assert!(!TenantTier::Free.uses_turbo_acceleration());
        assert!(!TenantTier::Standard.uses_turbo_acceleration());

        assert!(TenantTier::Turbo.uses_turbo_acceleration());
        assert!(TenantTier::Pro.uses_turbo_acceleration());
        assert!(TenantTier::Enterprise.uses_turbo_acceleration());
    }

    #[test]
    fn sentinel_monitoring_feature_flag() {
        assert!(!TenantTier::Free.has_sentinel_monitoring());
        assert!(!TenantTier::Standard.has_sentinel_monitoring());
        assert!(!TenantTier::Turbo.has_sentinel_monitoring());
        assert!(!TenantTier::Pro.has_sentinel_monitoring());
        assert!(TenantTier::Enterprise.has_sentinel_monitoring());
    }

    #[test]
    fn tier_parsing() {
        assert_eq!(TenantTier::from_u8(0), Some(TenantTier::Free));
        assert_eq!(TenantTier::from_u8(2), Some(TenantTier::Turbo));
        assert_eq!(TenantTier::from_u8(4), Some(TenantTier::Enterprise));
        assert_eq!(TenantTier::from_u8(5), None);
        assert_eq!(TenantTier::from_u8(255), None);
    }

    #[test]
    fn tier_names() {
        assert_eq!(TenantTier::Free.name(), "Free");
        assert_eq!(TenantTier::Turbo.name(), "Turbo");
        assert_eq!(format!("{}", TenantTier::Enterprise), "Enterprise");
    }

    #[test]
    fn upgrade_validation() {
        let free = TenantTier::Free;
        let standard = TenantTier::Standard;
        let turbo = TenantTier::Turbo;

        assert!(free.can_upgrade_to(standard));
        assert!(free.can_upgrade_to(turbo));
        assert!(standard.can_upgrade_to(TenantTier::Enterprise));

        assert!(!turbo.can_upgrade_to(standard));
        assert!(!standard.can_upgrade_to(free));
        assert!(!turbo.can_upgrade_to(turbo));
    }

    #[test]
    fn tier_progression() {
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

    #[test]
    fn default_tier() {
        assert_eq!(TenantTier::default(), TenantTier::Free);
    }

    #[test]
    fn serialization() {
        use serde_json;

        let tier = TenantTier::Turbo;
        let json = serde_json::to_string(&tier).unwrap();
        assert!(json.contains("Turbo"));

        let deserialized: TenantTier = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, TenantTier::Turbo);
    }

    #[test]
    fn tier_ordering() {
        assert!(TenantTier::Free < TenantTier::Enterprise);
        assert!(TenantTier::Pro > TenantTier::Standard);
        assert!(TenantTier::Turbo >= TenantTier::Turbo);
    }
}
