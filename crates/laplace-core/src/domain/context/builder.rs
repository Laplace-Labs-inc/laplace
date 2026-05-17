//! # Context Builder
//!
//! Fluent API for ergonomic context construction with validation.
//! Enables safe context creation with automatic tier selection and
//! intelligent defaults for turbo mode compatibility.

use laplace_interfaces::domain::{PriorityLevel, TenantTier};
use laplace_interfaces::domain::{SovereignContext, NO_TURBO_SLOT};

/// Context builder for ergonomic context construction.
///
/// Provides a fluent API for creating valid contexts with explicit tier
/// and priority settings. The builder ensures that invalid state transitions
/// cannot occur by automatically adjusting tier when turbo mode is enabled
/// on incompatible tiers.
///
/// # Examples
///
/// Creating a standard context:
///
/// ```ignore
/// let ctx = ContextBuilder::new("req-789")
///     .tenant("tenant-3")
///     .trace("trace-xyz")
///     .build();
/// assert!(ctx.is_valid());
/// ```
///
/// Creating an Enterprise turbo-mode context:
///
/// ```ignore
/// let ctx = ContextBuilder::new("req-789")
///     .tenant("tenant-3")
///     .trace("trace-xyz")
///     .tier(TenantTier::Enterprise)
///     .priority(PriorityLevel::Critical)
///     .turbo(true)
///     .build();
/// assert!(ctx.is_valid());
/// assert!(ctx.is_turbo_mode);
/// ```
pub struct ContextBuilder {
    request_id: String,
    tenant_id: String,
    trace_id: String,
    priority: u8,
    tier: u8,
    is_turbo_mode: bool,
}

impl ContextBuilder {
    /// Create new builder with request ID.
    ///
    /// Initializes the builder with:
    /// - priority: High (3)
    /// - tier: Standard (1)
    /// - is_turbo_mode: false
    ///
    /// # Arguments
    ///
    /// * `request_id` - Unique request identifier
    ///
    /// # Returns
    ///
    /// New builder instance for fluent chaining.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let builder = ContextBuilder::new("req-123");
    /// ```
    pub fn new(request_id: impl Into<String>) -> Self {
        Self {
            request_id: request_id.into(),
            tenant_id: String::new(),
            trace_id: String::new(),
            priority: PriorityLevel::High.as_u8(),
            tier: TenantTier::Standard.as_u8(),
            is_turbo_mode: false,
        }
    }

    /// Set tenant ID.
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Multi-tenant isolation scope
    ///
    /// # Returns
    ///
    /// Modified builder for fluent chaining.
    pub fn tenant(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = tenant_id.into();
        self
    }

    /// Set trace ID.
    ///
    /// # Arguments
    ///
    /// * `trace_id` - Distributed trace correlation identifier
    ///
    /// # Returns
    ///
    /// Modified builder for fluent chaining.
    pub fn trace(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = trace_id.into();
        self
    }

    /// Set priority level.
    ///
    /// # Arguments
    ///
    /// * `priority` - Priority level enum
    ///
    /// # Returns
    ///
    /// Modified builder for fluent chaining.
    pub fn priority(mut self, priority: PriorityLevel) -> Self {
        self.priority = priority.as_u8();
        self
    }

    /// Set tenant tier.
    ///
    /// Automatically adjusts turbo mode compatibility.
    /// If turbo mode is enabled and the new tier does not support it,
    /// turbo mode is disabled.
    ///
    /// # Arguments
    ///
    /// * `tier` - Tenant service tier
    ///
    /// # Returns
    ///
    /// Modified builder for fluent chaining.
    pub fn tier(mut self, tier: TenantTier) -> Self {
        self.tier = tier.as_u8();
        // Auto-adjust turbo mode compatibility
        if tier.supports_turbo() && self.is_turbo_mode {
            // Keep turbo mode enabled if tier supports it
        } else if !tier.supports_turbo() {
            // Disable turbo mode if tier doesn't support it
            self.is_turbo_mode = false;
        }
        self
    }

    /// Enable or disable turbo mode.
    ///
    /// If turbo mode is enabled on a tier that does not support it,
    /// the builder automatically upgrades the tier to Turbo.
    ///
    /// This ensures the built context is always valid without requiring
    /// the caller to manually manage tier-turbo compatibility.
    ///
    /// # Arguments
    ///
    /// * `enabled` - Whether to enable turbo-mode acceleration
    ///
    /// # Returns
    ///
    /// Modified builder for fluent chaining.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Turbo enabled on Free tier → builder auto-upgrades to Turbo tier
    /// let ctx = ContextBuilder::new("req-123")
    ///     .tenant("t1")
    ///     .trace("tr")
    ///     .tier(TenantTier::Free)
    ///     .turbo(true)
    ///     .build();
    /// assert!(ctx.tenant_tier().unwrap().supports_turbo());
    /// ```
    pub fn turbo(mut self, enabled: bool) -> Self {
        self.is_turbo_mode = enabled;
        // Validate compatibility with tier
        if enabled {
            if let Some(tier) = TenantTier::from_u8(self.tier) {
                if !tier.supports_turbo() {
                    // Automatically upgrade to Turbo tier
                    self.tier = TenantTier::Turbo.as_u8();
                }
            }
        }
        self
    }

    /// Build the context.
    ///
    /// Constructs a validated SovereignContext. In debug builds, asserts
    /// that the context is valid. In release builds, the context may be
    /// invalid if inputs were malformed, though the builder design makes
    /// this unlikely.
    ///
    /// # Returns
    ///
    /// Validated SovereignContext instance.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if the built context is invalid.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let ctx = ContextBuilder::new("req-123")
    ///     .tenant("tenant-1")
    ///     .trace("trace-xyz")
    ///     .build();
    /// ```
    pub fn build(self) -> SovereignContext {
        let ctx = SovereignContext {
            request_id: self.request_id,
            tenant_id: self.tenant_id,
            trace_id: self.trace_id,
            priority: self.priority,
            tier: self.tier,
            is_turbo_mode: self.is_turbo_mode,
            timestamp: SovereignContext::current_timestamp(),
            turbo_slot: NO_TURBO_SLOT,
        };
        debug_assert!(ctx.is_valid(), "Built context must be valid: {}", ctx);
        ctx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_builder_flow() {
        let ctx = ContextBuilder::new("req-789")
            .tenant("tenant-3")
            .trace("trace-xyz")
            .tier(TenantTier::Enterprise)
            .priority(PriorityLevel::Critical)
            .turbo(true)
            .build();
        assert!(ctx.is_valid());
        assert!(ctx.is_turbo_mode);
        assert_eq!(ctx.request_id, "req-789");
        assert_eq!(ctx.tier, 4); // Enterprise
        assert_eq!(ctx.priority, 4); // Critical
    }

    #[test]
    fn context_invalid_turbo_without_tier() {
        let ctx = ContextBuilder::new("req-bad")
            .tenant("t1")
            .trace("tr")
            .tier(TenantTier::Free)
            .turbo(true) // Incompatible!
            .build();
        // Builder should auto-upgrade tier to support turbo
        assert!(ctx.tenant_tier().unwrap().supports_turbo());
        assert_eq!(ctx.tier, TenantTier::Turbo.as_u8());
    }

    #[test]
    fn context_builder_minimal() {
        let ctx = ContextBuilder::new("req-min")
            .tenant("t1")
            .trace("tr")
            .build();
        assert!(ctx.is_valid());
        assert_eq!(ctx.priority, PriorityLevel::High.as_u8());
        assert_eq!(ctx.tier, TenantTier::Standard.as_u8());
    }

    #[test]
    fn context_builder_turbo_upgrade() {
        // Turbo on Standard tier should auto-upgrade
        let ctx = ContextBuilder::new("req-upgrade")
            .tenant("tenant-upgrade")
            .trace("trace-upgrade")
            .tier(TenantTier::Standard)
            .turbo(true)
            .build();
        assert!(ctx.is_turbo_mode);
        assert!(ctx.is_valid());
        assert_eq!(ctx.tier, TenantTier::Turbo.as_u8());
    }

    #[test]
    fn context_builder_turbo_on_pro() {
        // Turbo on Pro tier should keep Pro tier
        let ctx = ContextBuilder::new("req-pro")
            .tenant("tenant-pro")
            .trace("trace-pro")
            .tier(TenantTier::Pro)
            .turbo(true)
            .build();
        assert!(ctx.is_turbo_mode);
        assert!(ctx.is_valid());
        assert_eq!(ctx.tier, TenantTier::Pro.as_u8());
    }

    #[test]
    fn context_builder_priority_levels() {
        for level in 0..=5 {
            let ctx = ContextBuilder::new("req-priority")
                .tenant("t")
                .trace("tr")
                .priority(PriorityLevel::from_u8(level).unwrap())
                .build();
            assert_eq!(ctx.priority, level);
            assert!(ctx.is_valid());
        }
    }

    #[test]
    fn context_builder_all_tiers() {
        let tiers = [
            TenantTier::Free,
            TenantTier::Standard,
            TenantTier::Turbo,
            TenantTier::Pro,
            TenantTier::Enterprise,
        ];
        for tier in &tiers {
            let ctx = ContextBuilder::new("req-tier")
                .tenant("t")
                .trace("tr")
                .tier(*tier)
                .build();
            assert_eq!(ctx.tier, tier.as_u8());
        }
    }

    #[test]
    fn context_builder_turbo_disabled_on_incompatible_tier() {
        // Setting tier to Free with turbo already enabled should disable turbo
        let ctx = ContextBuilder::new("req-disable")
            .tenant("t")
            .trace("tr")
            .turbo(true) // Enable turbo first
            .tier(TenantTier::Free) // Then set incompatible tier
            .build();
        assert!(!ctx.is_turbo_mode);
        assert!(ctx.is_valid());
    }
}
