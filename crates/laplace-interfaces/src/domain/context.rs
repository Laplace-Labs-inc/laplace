//! # Sovereign Context
//!
//! Core context structure that flows through the entire Laplace stack.
//! Implements the Deterministic Context principle: all async operations
//! and domain logic receive an explicit `ctx: SovereignContext` parameter.
//!
//! This is the Single Source of Truth (SSOT) for context definitions.
//! All other representations (Protobuf, FFI, TypeScript) are derived from
//! this Rust structure.

use super::scheduling::PriorityLevel;
use super::tenant::TenantTier;
use serde::{Deserialize, Serialize};
use std::fmt;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

// Time imports only used in non-Kani mode
#[cfg(not(kani))]
use std::time::{SystemTime, UNIX_EPOCH};

/// Sentinel value indicating no turbo slot allocation.
///
/// When `turbo_slot == NO_TURBO_SLOT`, the context is using standard FFI
/// and does not have a physical slot in the shared memory pool.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Context",
        link = "LEP-0008-laplace-interfaces-sovereign_context"
    )
)]
pub const NO_TURBO_SLOT: u32 = u32::MAX;

/// Sovereign Context: the canonical context object passed through all kernel operations.
///
/// Ensures complete traceability, deterministic behavior, and proper request scoping.
/// Must be explicitly threaded through async operations and domain functions.
///
/// This structure is the Single Source of Truth (SSOT) for all Laplace context definitions.
/// All other representations (Protobuf, FFI, TypeScript) are derived from this definition.
///
/// # Memory Layout (FFI Compatible)
///
/// When serialized for FFI:
/// - request_id: String → pointer + length
/// - tenant_id: String → pointer + length
/// - trace_id: String → pointer + length
/// - priority: u8 (1 byte)
/// - tier: u8 (1 byte)
/// - is_turbo_mode: bool (1 byte)
/// - timestamp: u64 (8 bytes)
/// - turbo_slot: u32 (4 bytes)
///
/// Total variable-length data: request_id + tenant_id + trace_id
///
/// # Examples
///
/// Creating a standard context:
///
/// ```ignore
/// let ctx = SovereignContext::new(
///     "req-123".to_string(),
///     "tenant-acme".to_string(),
///     "trace-xyz".to_string(),
/// );
/// ```
///
/// Creating a turbo-mode context for premium tenant:
///
/// ```ignore
/// let turbo_ctx = SovereignContext::new_turbo(
///     "req-456".to_string(),
///     "tenant-premium".to_string(),
///     "trace-fast".to_string(),
/// ).with_tier(TenantTier::Enterprise);
/// ```
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Context",
        link = "LEP-0008-laplace-interfaces-sovereign_context"
    )
)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SovereignContext {
    /// Unique request identifier (UUID or snowflake).
    ///
    /// Used for request deduplication, tracing, and logging.
    /// Must be non-empty for valid context.
    pub request_id: String,

    /// Multi-tenant isolation scope.
    ///
    /// Determines tenant billing and resource boundaries.
    /// Must be non-empty for valid context.
    pub tenant_id: String,

    /// Distributed trace correlation ID.
    ///
    /// Propagated through all child operations for observability.
    /// Must be non-empty for valid context.
    pub trace_id: String,

    /// Request priority level for scheduling decisions.
    ///
    /// Range: 0-5 (Lowest to SystemCritical).
    /// Default: 3 (High).
    /// Used by kernel scheduler under resource contention.
    pub priority: u8,

    /// Tenant service tier (Free/Standard/Turbo/Pro/Enterprise).
    ///
    /// Determines resource limits and feature availability.
    /// Range: 0-4.
    /// Default: 1 (Standard).
    pub tier: u8,

    /// Turbo-mode flag: true = shared memory FFI, false = standard FFI.
    ///
    /// Only valid when tier >= 2 (Turbo or higher).
    /// When enabled, enables zero-copy acceleration via shared memory pool.
    pub is_turbo_mode: bool,

    /// Request creation timestamp (Unix nanoseconds since epoch).
    ///
    /// Used for latency measurement and timeout calculation.
    pub timestamp: u64,

    /// Physical slot index in shared memory pool (Turbo mode only).
    ///
    /// Semantics:
    /// - u32::MAX (0xFFFFFFFF) = Not allocated (standard FFI).
    /// - 0-N = Allocated turbo slot index.
    ///
    /// Only meaningful when is_turbo_mode == true.
    pub turbo_slot: u32,
}

impl SovereignContext {
    /// Create a new context with standard FFI settings.
    ///
    /// Initializes the context with:
    /// - priority: High (3)
    /// - tier: Standard (1)
    /// - turbo_slot: Not allocated (NO_TURBO_SLOT)
    /// - is_turbo_mode: false
    ///
    /// # Arguments
    ///
    /// * `request_id` - Unique request identifier
    /// * `tenant_id` - Multi-tenant isolation scope
    /// * `trace_id` - Distributed trace correlation ID
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let ctx = SovereignContext::new(
    ///     "req-123".to_string(),
    ///     "tenant-1".to_string(),
    ///     "trace-abc".to_string(),
    /// );
    /// assert!(ctx.is_valid());
    /// ```
    pub fn new(request_id: String, tenant_id: String, trace_id: String) -> Self {
        Self {
            request_id,
            tenant_id,
            trace_id,
            priority: PriorityLevel::High.as_u8(),
            tier: TenantTier::Standard.as_u8(),
            is_turbo_mode: false,
            timestamp: Self::current_timestamp(),
            turbo_slot: NO_TURBO_SLOT,
        }
    }

    /// Create a context in turbo mode (zero-copy shared memory).
    ///
    /// Initializes the context with:
    /// - priority: High (3)
    /// - tier: Turbo (2)
    /// - is_turbo_mode: true
    /// - turbo_slot: Not allocated (will be assigned by kernel)
    ///
    /// # Arguments
    ///
    /// * `request_id` - Unique request identifier
    /// * `tenant_id` - Multi-tenant isolation scope
    /// * `trace_id` - Distributed trace correlation ID
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let ctx = SovereignContext::new_turbo(
    ///     "req-456".to_string(),
    ///     "tenant-2".to_string(),
    ///     "trace-def".to_string(),
    /// );
    /// assert!(ctx.is_valid());
    /// assert!(ctx.is_turbo_mode);
    /// ```
    pub fn new_turbo(request_id: String, tenant_id: String, trace_id: String) -> Self {
        Self {
            request_id,
            tenant_id,
            trace_id,
            priority: PriorityLevel::High.as_u8(),
            tier: TenantTier::Turbo.as_u8(),
            is_turbo_mode: true,
            timestamp: Self::current_timestamp(),
            turbo_slot: NO_TURBO_SLOT, // Will be assigned by kernel
        }
    }

    /// Set priority level using builder pattern.
    ///
    /// Clamps the value to the valid range [0, 5].
    ///
    /// # Arguments
    ///
    /// * `priority` - Priority level (0-5)
    ///
    /// # Returns
    ///
    /// Modified context for fluent chaining.
    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority.min(5); // Clamp to valid range
        self
    }

    /// Set tenant tier using builder pattern.
    ///
    /// Automatically disables turbo mode if the tier does not support it.
    ///
    /// # Arguments
    ///
    /// * `tier` - Tenant service tier
    ///
    /// # Returns
    ///
    /// Modified context for fluent chaining.
    pub fn with_tier(mut self, tier: TenantTier) -> Self {
        self.tier = tier.as_u8();
        // Disable turbo if tier doesn't support it
        if !tier.uses_turbo_acceleration() {
            self.is_turbo_mode = false;
        }
        self
    }

    /// Allocate turbo slot (kernel only).
    ///
    /// Assigns a physical slot index in the shared memory pool.
    /// This is typically called by the kernel after validating the context.
    ///
    /// # Arguments
    ///
    /// * `slot` - Physical slot index in shared memory pool
    ///
    /// # Returns
    ///
    /// Modified context for fluent chaining.
    pub fn allocate_turbo_slot(mut self, slot: u32) -> Self {
        self.turbo_slot = slot;
        self
    }

    /// Create a child context for spawned operations.
    ///
    /// Inherits tier, priority, and trace_id from parent.
    /// Assigns new request_id and timestamp.
    /// Child does not inherit turbo_slot; it will be allocated separately if needed.
    ///
    /// # Arguments
    ///
    /// * `new_request_id` - Unique identifier for the child operation
    ///
    /// # Returns
    ///
    /// New context with parent's SLA and trace information.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let parent = SovereignContext::new(...);
    /// let child = parent.spawn_child("child-req-123".to_string());
    /// assert_eq!(child.trace_id, parent.trace_id);
    /// assert_ne!(child.request_id, parent.request_id);
    /// ```
    pub fn spawn_child(&self, new_request_id: String) -> Self {
        Self {
            request_id: new_request_id,
            tenant_id: self.tenant_id.clone(),
            trace_id: self.trace_id.clone(),
            priority: self.priority,
            tier: self.tier,
            is_turbo_mode: self.is_turbo_mode,
            timestamp: Self::current_timestamp(),
            turbo_slot: NO_TURBO_SLOT, // Child gets new slot if needed
        }
    }

    /// Get current system timestamp in nanoseconds.
    ///
    /// Returns Unix epoch time in nanoseconds.
    /// In Kani verification mode, returns a fixed value to avoid system time dependencies.
    pub fn current_timestamp() -> u64 {
        #[cfg(kani)]
        {
            // In Kani verification mode, return fixed timestamp to avoid clock_gettime
            1_000_000_000_000_000
        }
        #[cfg(not(kani))]
        {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0)
        }
    }

    /// Check if context is valid for request processing.
    ///
    /// Validates:
    /// - Non-empty identifiers (request_id, tenant_id, trace_id)
    /// - Priority in range [0, 5]
    /// - Tier in range [0, 4]
    /// - Turbo mode consistent with tier support
    ///
    /// # Returns
    ///
    /// `true` if context is valid, `false` otherwise.
    pub fn is_valid(&self) -> bool {
        let has_ids =
            !self.request_id.is_empty() && !self.tenant_id.is_empty() && !self.trace_id.is_empty();

        let priority_valid = self.priority <= 5;
        let tier_valid = self.tier <= 4;

        let turbo_consistent = {
            if self.is_turbo_mode {
                // Turbo mode requires Turbo tier or higher
                let tier = TenantTier::from_u8(self.tier);
                tier.map(|t| t.uses_turbo_acceleration()).unwrap_or(false)
            } else {
                true // Standard FFI always valid
            }
        };

        has_ids && priority_valid && tier_valid && turbo_consistent
    }

    /// Elapsed time since context creation in nanoseconds.
    ///
    /// # Returns
    ///
    /// Nanoseconds elapsed since the context timestamp.
    /// Uses saturating subtraction to avoid underflow on clock skew.
    pub fn elapsed_ns(&self) -> u64 {
        Self::current_timestamp().saturating_sub(self.timestamp)
    }

    /// Check if turbo slot is allocated.
    ///
    /// # Returns
    ///
    /// `true` if a turbo slot has been allocated, `false` otherwise.
    #[inline]
    pub fn has_turbo_slot(&self) -> bool {
        self.turbo_slot != NO_TURBO_SLOT
    }

    /// Get turbo slot if allocated.
    ///
    /// # Returns
    ///
    /// `Some(slot_index)` if turbo slot is allocated, `None` otherwise.
    pub fn get_turbo_slot(&self) -> Option<u32> {
        if self.turbo_slot != NO_TURBO_SLOT {
            Some(self.turbo_slot)
        } else {
            None
        }
    }

    /// Get priority level as enum.
    ///
    /// # Returns
    ///
    /// Priority level enum if valid, `None` if out of range.
    pub fn priority_level(&self) -> Option<PriorityLevel> {
        PriorityLevel::from_u8(self.priority)
    }

    /// Get tier as enum.
    ///
    /// # Returns
    ///
    /// Tenant tier enum if valid, `None` if out of range.
    pub fn tenant_tier(&self) -> Option<TenantTier> {
        TenantTier::from_u8(self.tier)
    }
}

impl fmt::Display for SovereignContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tier_str = self
            .tenant_tier()
            .map(|t| format!("{:?}", t))
            .unwrap_or_else(|| format!("Invalid({})", self.tier));
        let priority_str = self
            .priority_level()
            .map(|p| format!("{:?}", p))
            .unwrap_or_else(|| format!("Invalid({})", self.priority));

        write!(
            f,
            "SovereignContext {{ req_id: {}, tenant: {}, trace: {}, priority: {}, tier: {}, turbo: {}, turbo_slot: {}, elapsed: {}ns }}",
            self.request_id,
            self.tenant_id,
            self.trace_id,
            priority_str,
            tier_str,
            self.is_turbo_mode,
            if self.has_turbo_slot() {
                format!("{}", self.turbo_slot)
            } else {
                "None".to_string()
            },
            self.elapsed_ns()
        )
    }
}

impl Default for SovereignContext {
    fn default() -> Self {
        Self {
            request_id: "default".to_string(),
            tenant_id: "default".to_string(),
            trace_id: "default".to_string(),
            priority: PriorityLevel::Normal.as_u8(),
            tier: TenantTier::Standard.as_u8(),
            is_turbo_mode: false,
            timestamp: 0,
            turbo_slot: NO_TURBO_SLOT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sovereign_context_creation() {
        let ctx = SovereignContext::new(
            "req-123".to_string(),
            "tenant-1".to_string(),
            "trace-abc".to_string(),
        );
        assert!(ctx.is_valid());
        assert!(!ctx.is_turbo_mode);
        assert_eq!(ctx.priority, 3); // High
        assert_eq!(ctx.tier, 1); // Standard
        assert!(!ctx.has_turbo_slot());
    }

    #[test]
    fn sovereign_context_turbo() {
        let ctx = SovereignContext::new_turbo(
            "req-456".to_string(),
            "tenant-2".to_string(),
            "trace-def".to_string(),
        );
        assert!(ctx.is_valid());
        assert!(ctx.is_turbo_mode);
        assert_eq!(ctx.tier, 2); // Turbo
    }

    #[test]
    fn context_spawn_child() {
        let parent = SovereignContext::new(
            "parent".to_string(),
            "t1".to_string(),
            "trace-1".to_string(),
        )
        .with_priority(4)
        .with_tier(TenantTier::Pro);

        let child = parent.spawn_child("child".to_string());
        assert_eq!(child.tenant_id, parent.tenant_id);
        assert_eq!(child.trace_id, parent.trace_id);
        assert_eq!(child.priority, parent.priority);
        assert_eq!(child.tier, parent.tier);
        assert_ne!(child.request_id, parent.request_id);
        assert!(!child.has_turbo_slot());
    }

    #[test]
    fn context_serialization() {
        let ctx = SovereignContext::new(
            "req-ser".to_string(),
            "tenant-ser".to_string(),
            "trace-ser".to_string(),
        )
        .with_tier(TenantTier::Enterprise);

        let json = serde_json::to_string(&ctx).expect("should serialize");
        let deserialized: SovereignContext =
            serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(ctx.request_id, deserialized.request_id);
        assert_eq!(ctx.tier, deserialized.tier);
    }

    #[test]
    fn context_validity_checks() {
        // Valid context
        let valid =
            SovereignContext::new("req".to_string(), "tenant".to_string(), "trace".to_string());
        assert!(valid.is_valid());

        // Invalid: empty request_id
        let mut invalid = valid.clone();
        invalid.request_id = String::new();
        assert!(!invalid.is_valid());

        // Invalid: priority out of range
        let mut invalid = valid.clone();
        invalid.priority = 6;
        assert!(!invalid.is_valid());

        // Invalid: tier out of range
        let mut invalid = valid.clone();
        invalid.tier = 5;
        assert!(!invalid.is_valid());

        // Invalid: turbo mode without turbo tier
        let mut invalid = valid.clone();
        invalid.is_turbo_mode = true;
        invalid.tier = TenantTier::Free.as_u8();
        assert!(!invalid.is_valid());
    }

    #[test]
    fn turbo_slot_management() {
        let mut ctx = SovereignContext::new_turbo(
            "req".to_string(),
            "tenant".to_string(),
            "trace".to_string(),
        );
        assert!(!ctx.has_turbo_slot());
        assert_eq!(ctx.get_turbo_slot(), None);

        ctx = ctx.allocate_turbo_slot(42);
        assert!(ctx.has_turbo_slot());
        assert_eq!(ctx.get_turbo_slot(), Some(42));
    }
}
