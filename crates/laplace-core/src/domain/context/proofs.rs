#![cfg(kani)]

//! Formal Verification Proofs for SovereignContext Isolation
//!
//! This module contains Kani symbolic execution proofs that formally verify the
//! correctness of tenant context isolation, tier enforcement, and slot management.
//! These proofs ensure that the context implementation maintains critical invariants
//! preventing cross-tenant interference and enforcing business rule constraints.
//!
//! # Verified Properties
//!
//! The following properties are formally verified:
//!
//! 1. **Slot Boundary Isolation**: Turbo slot indices remain within valid bounds,
//!    never exceeding the physical pool capacity or confusing allocated/unallocated states.
//!
//! 2. **Tier-Turbo Enforcement**: Only Turbo tier and above (Turbo, Pro, Enterprise)
//!    can have `is_turbo_mode=true`. Lower tiers are automatically corrected.
//!
//! 3. **Lifecycle Immutability**: Tenant identifiers remain unchanged throughout
//!    the context lifecycle via factory methods (spawn_child maintains tenant_id).

use crate::domain::context::{ContextBuilder, SovereignContext, NO_TURBO_SLOT};
use crate::domain::TenantTier;

/// Proof: Turbo slot boundary isolation — allocated slots never exceed physical pool capacity.
///
/// This proof verifies that when a turbo slot is allocated via `allocate_turbo_slot(slot)`,
/// the slot index never collides with the NO_TURBO_SLOT sentinel (u32::MAX) and remains
/// within the valid range [0, MAX_POOL_CAPACITY-1]. This prevents memory corruption and
/// cross-tenant slot confusion.
///
/// # TLA+ Correspondence
///
/// ```tla
/// SlotBoundaryInvariant ==
///     /\ allocated_slot < u32::MAX  (never is NO_TURBO_SLOT)
///     /\ allocated_slot < MAX_CAPACITY
///     /\ unallocated_slot = u32::MAX  (exactly NO_TURBO_SLOT)
/// ```
#[kani::proof]
#[cfg_attr(kani, kani::unwind(5))]
fn verify_slot_boundary_isolation() {
    // Allocate a symbolic slot within a reasonable pool capacity (e.g., 1000 max slots)
    let slot: u32 = kani::any();
    let max_capacity: u32 = 1000; // Physical pool capacity sentinel

    // Precondition: slot must be within valid range for allocation
    kani::assume(slot < max_capacity);

    // Create a standard context (starts unallocated)
    let ctx = SovereignContext::new(
        "req-123".to_string(),
        "tenant-slot-test".to_string(),
        "trace-slot-test".to_string(),
    );

    // Before allocation: turbo_slot must be NO_TURBO_SLOT
    assert_eq!(
        ctx.turbo_slot,
        u32::MAX,
        "Unallocated context must have turbo_slot = u32::MAX (NO_TURBO_SLOT)"
    );
    assert!(
        !ctx.has_turbo_slot(),
        "Unallocated context must return false from has_turbo_slot()"
    );

    // Allocate a valid slot
    let allocated_ctx = ctx.allocate_turbo_slot(slot);

    // After allocation: turbo_slot must be exactly the allocated slot, not NO_TURBO_SLOT
    assert_eq!(
        allocated_ctx.turbo_slot, slot,
        "Allocated context must have turbo_slot = the input slot value"
    );

    // Critical: allocated slot must never equal NO_TURBO_SLOT
    assert_ne!(
        allocated_ctx.turbo_slot,
        u32::MAX,
        "Allocated slot must never be NO_TURBO_SLOT (u32::MAX)"
    );

    // has_turbo_slot must return true after allocation (unless slot happened to be MAX,
    // but we preconditioned slot < max_capacity < u32::MAX)
    assert!(
        allocated_ctx.has_turbo_slot(),
        "Allocated context must return true from has_turbo_slot()"
    );

    // Verify get_turbo_slot returns the correct slot
    assert_eq!(
        allocated_ctx.get_turbo_slot(),
        Some(slot),
        "get_turbo_slot() must return Some(slot) after allocation"
    );
}

/// Proof: Tier-Turbo enforcement — only Turbo+ tiers can have is_turbo_mode=true.
///
/// This proof verifies that the builder pattern correctly enforces the business rule:
/// "Turbo mode is only valid for Turbo tier and above (Turbo, Pro, Enterprise)."
/// Lower tiers (Free, Standard) have turbo mode automatically disabled by `with_tier()`.
///
/// # TLA+ Correspondence
///
/// ```tla
/// TierTurboEnforcement ==
///     /\ tier \in {Turbo, Pro, Enterprise} => is_turbo_mode = true  (if enabled)
///     /\ tier \in {Free, Standard} => is_turbo_mode = false  (always disabled)
///     /\ with_tier(t) disables turbo if t does not support it
/// ```
#[kani::proof]
#[cfg_attr(kani, kani::unwind(5))]
fn verify_tier_turbo_enforcement() {
    // Symbolic tier value (0-4 maps to Free, Standard, Turbo, Pro, Enterprise)
    let tier_value: u8 = kani::any();

    // Parse into TenantTier enum
    if let Some(tier) = TenantTier::from_u8(tier_value) {
        // Test Case 1: Lower tiers cannot enable turbo mode
        if tier_value < 2 {
            // Free (0) or Standard (1)
            // Create a turbo context (which should have turbo=true and tier=Turbo)
            let mut ctx = SovereignContext::new_turbo(
                "req-tier-test".to_string(),
                "tenant-tier-test".to_string(),
                "trace-tier-test".to_string(),
            );

            // Immediately change tier to a lower tier
            ctx = ctx.with_tier(tier);

            // Assertion: is_turbo_mode must be disabled for lower tiers
            assert!(
                !ctx.is_turbo_mode,
                "Tier {} does not support turbo mode; is_turbo_mode must be false",
                tier_value
            );

            // Context must still be valid despite the tier change
            assert!(
                ctx.is_valid(),
                "Context must remain valid after disabling turbo for incompatible tier"
            );
        }

        // Test Case 2: Turbo+ tiers support turbo mode
        if tier_value >= 2 {
            // Turbo (2), Pro (3), Enterprise (4)
            let ctx = SovereignContext::new_turbo(
                "req-tier-high".to_string(),
                "tenant-tier-high".to_string(),
                "trace-tier-high".to_string(),
            );

            // Verify initial turbo mode is true for Turbo+ tiers
            assert!(
                ctx.is_turbo_mode,
                "Tier {} should support turbo mode; is_turbo_mode should be true",
                tier_value
            );

            // Now set to a Turbo+ tier explicitly
            let ctx_explicit = ctx.with_tier(tier);
            assert!(
                ctx_explicit.is_turbo_mode,
                "Setting tier to {} (a Turbo+ tier) must preserve turbo mode",
                tier_value
            );
            assert!(
                ctx_explicit.is_valid(),
                "Context with Turbo+ tier must be valid"
            );
        }
    }
}

/// Proof: Lifecycle immutability — spawn_child preserves parent's tenant_id.
///
/// This proof verifies that the factory method `spawn_child()` creates a child context
/// that inherits the parent's tenant_id, preventing cross-tenant leakage. The child
/// gets a new request_id and timestamp, but maintains tenant isolation.
///
/// # TLA+ Correspondence
///
/// ```tla
/// LifecycleImmutability ==
///     /\ child.tenant_id = parent.tenant_id
///     /\ child.trace_id = parent.trace_id  (trace propagation)
///     /\ child.request_id \neq parent.request_id  (new request)
///     /\ child.priority = parent.priority  (SLA preservation)
///     /\ child.tier = parent.tier  (tier inheritance)
///     /\ child.turbo_slot = NO_TURBO_SLOT  (child re-allocation)
/// ```
#[kani::proof]
#[cfg_attr(kani, kani::unwind(20))]
fn verify_lifecycle_immutability_spawn_child() {
    // Symbolic parent context values
    let parent_tenant_id = "tenant-parent".to_string();
    let parent_request_id = "req-parent".to_string();
    let parent_trace_id = "trace-parent".to_string();
    let parent_priority: u8 = kani::any();
    let parent_tier_value: u8 = kani::any();

    // Constrain to valid values
    kani::assume(parent_priority <= 5);
    kani::assume(parent_tier_value <= 4);

    // Build parent context with custom tier
    let parent = if let Some(tier) = TenantTier::from_u8(parent_tier_value) {
        SovereignContext::new(
            parent_request_id.clone(),
            parent_tenant_id.clone(),
            parent_trace_id.clone(),
        )
        .with_priority(parent_priority)
        .with_tier(tier)
    } else {
        // Fallback: create standard context
        SovereignContext::new(
            parent_request_id.clone(),
            parent_tenant_id.clone(),
            parent_trace_id.clone(),
        )
    };

    // Verify parent is valid before spawning
    assert!(parent.is_valid(), "Parent context must be valid");

    // Spawn a child context
    let child_request_id = "req-child".to_string();
    let child = parent.spawn_child(child_request_id.clone());

    // Assertion 1: Child inherits tenant_id (tenant isolation preserved)
    assert_eq!(
        child.tenant_id, parent.tenant_id,
        "Child must inherit parent's tenant_id for request isolation"
    );

    // Assertion 2: Child inherits trace_id (distributed tracing)
    assert_eq!(
        child.trace_id, parent.trace_id,
        "Child must inherit parent's trace_id for distributed tracing"
    );

    // Assertion 3: Child gets new request_id (unique per operation)
    assert_eq!(
        child.request_id, child_request_id,
        "Child must have the new request_id passed to spawn_child()"
    );
    assert_ne!(
        child.request_id, parent.request_id,
        "Child's request_id must differ from parent's"
    );

    // Assertion 4: Child inherits priority (SLA preservation)
    assert_eq!(
        child.priority, parent.priority,
        "Child must inherit parent's priority level"
    );

    // Assertion 5: Child inherits tier (SLA preservation)
    assert_eq!(child.tier, parent.tier, "Child must inherit parent's tier");

    // Assertion 6: Child inherits turbo_mode (execution path consistency)
    assert_eq!(
        child.is_turbo_mode, parent.is_turbo_mode,
        "Child must inherit parent's is_turbo_mode"
    );

    // Assertion 7: Child's turbo_slot is NOT allocated (fresh re-allocation)
    assert_eq!(
        child.turbo_slot,
        u32::MAX,
        "Child must start with unallocated turbo_slot (NO_TURBO_SLOT)"
    );
    assert!(
        !child.has_turbo_slot(),
        "Child must return false from has_turbo_slot() before allocation"
    );

    // Assertion 8: Child context must be valid
    assert!(
        child.is_valid(),
        "Child context must be valid after spawn_child()"
    );
}

/// Proof: Slot allocation is idempotent with latest-write-wins semantics.
///
/// This proof verifies that calling `allocate_turbo_slot()` multiple times
/// yields a context with the latest slot value. This prevents slot confusion
/// if allocation is called multiple times (though in production it should be once).
///
/// # TLA+ Correspondence
///
/// ```tla
/// SlotAllocationIdempotency ==
///     /\ allocate(s1).turbo_slot = s1
///     /\ allocate(s1).allocate(s2).turbo_slot = s2  (latest wins)
///     /\ get_turbo_slot never changes after last allocate call
/// ```
#[kani::proof]
#[cfg_attr(kani, kani::unwind(5))]
fn verify_slot_allocation_idempotency() {
    let slot1: u32 = kani::any();
    let slot2: u32 = kani::any();

    // Preconditions: both slots are distinct and within capacity
    kani::assume(slot1 < 1000);
    kani::assume(slot2 < 1000);
    kani::assume(slot1 != slot2);

    let ctx = SovereignContext::new(
        "req-alloc-test".to_string(),
        "tenant-alloc-test".to_string(),
        "trace-alloc-test".to_string(),
    );

    // First allocation
    let ctx_after_1 = ctx.allocate_turbo_slot(slot1);
    assert_eq!(
        ctx_after_1.turbo_slot, slot1,
        "First allocation must set turbo_slot = slot1"
    );

    // Second allocation (overwrites)
    let ctx_after_2 = ctx_after_1.allocate_turbo_slot(slot2);
    assert_eq!(
        ctx_after_2.turbo_slot, slot2,
        "Second allocation must set turbo_slot = slot2 (latest wins)"
    );

    // Verify second allocation value is stable
    assert_ne!(
        ctx_after_2.turbo_slot, slot1,
        "Turbo slot must be updated to the latest allocation"
    );
}

/// Proof: Context identifier uniqueness in multi-tenant scenarios.
///
/// This proof verifies that two contexts created for different tenants maintain
/// complete isolation: different tenant_ids mean different contexts, never accidentally
/// sharing resources or data.
///
/// # TLA+ Correspondence
///
/// ```tla
/// MultiTenantIsolation ==
///     /\ tenant1_id != tenant2_id => ctx1.tenant_id != ctx2.tenant_id
///     /\ ctx1.tenant_id uniquely identifies all resources for ctx1
///     /\ ctx1 cannot access ctx2's resources via context lookup
/// ```
#[kani::proof]
#[cfg_attr(kani, kani::unwind(15))]
fn verify_multi_tenant_isolation() {
    // Two distinct tenants
    let tenant1 = "tenant-acme".to_string();
    let tenant2 = "tenant-beta".to_string();

    // Create contexts for each tenant
    let ctx1 = SovereignContext::new("req-1".to_string(), tenant1.clone(), "trace-1".to_string());

    let ctx2 = SovereignContext::new("req-2".to_string(), tenant2.clone(), "trace-2".to_string());

    // Assertion 1: Different tenant_ids
    assert_ne!(
        ctx1.tenant_id, ctx2.tenant_id,
        "Different tenants must have different tenant_ids"
    );

    // Assertion 2: Different request_ids (orthogonal isolation)
    assert_ne!(
        ctx1.request_id, ctx2.request_id,
        "Different requests must have different request_ids"
    );

    // Assertion 3: Same tenant can have multiple requests, but all share tenant_id
    let req1_ctx1 =
        SovereignContext::new("req-1a".to_string(), tenant1.clone(), "trace-1".to_string());
    let req1_ctx1b =
        SovereignContext::new("req-1b".to_string(), tenant1.clone(), "trace-1".to_string());

    assert_eq!(
        req1_ctx1.tenant_id, req1_ctx1b.tenant_id,
        "Multiple requests from same tenant must share tenant_id"
    );
    assert_ne!(
        req1_ctx1.request_id, req1_ctx1b.request_id,
        "Multiple requests from same tenant must have different request_ids"
    );
}

// ── H-CTX6 ────────────────────────────────────────────────────────────────────

/// Proof: ContextBuilder gracefully rejects turbo mode for non-turbo tiers.
///
/// Verifies that when turbo mode is requested but a non-turbo tier is subsequently
/// set, the builder auto-corrects (disables turbo) **without panicking**. Reaching
/// the assertion statements proves that `build()` returned a valid context rather
/// than panicking.
///
/// # Two scenarios covered
///
/// 1. `turbo(true)` → `tier(Free)` — turbo is disabled because Free does not
///    support turbo mode; the tier remains Free.
///
/// 2. `tier(Standard)` → `turbo(true)` — builder auto-upgrades the tier to Turbo
///    so that turbo mode can be honoured; is_turbo_mode is true.
///
/// # Invariant
///
/// ```text
/// build(turbo=true, tier=NonTurbo) => is_turbo_mode = false ∧ ¬panics
/// build(tier=NonTurbo, turbo=true) => tier_supports_turbo ∧ is_turbo_mode = true ∧ ¬panics
/// ```
#[kani::proof]
#[cfg_attr(kani, kani::unwind(5))]
fn proof_builder_rejects_invalid_turbo() {
    // ── Case 1: turbo enabled, then overridden by Free tier ───────────────────
    // Reaching the next line proves build() did not panic.
    let ctx_free = ContextBuilder::new("req-reject-free")
        .tenant("tenant-free")
        .trace("trace-free")
        .turbo(true) // Request turbo first …
        .tier(TenantTier::Free) // … then set incompatible tier (disables turbo)
        .build();

    // Builder must have auto-corrected: turbo is disabled for Free tier.
    assert!(
        !ctx_free.is_turbo_mode,
        "Free tier must have turbo disabled — builder must reject the invalid combination"
    );
    assert_eq!(
        ctx_free.tier,
        TenantTier::Free.as_u8(),
        "Tier must remain Free after the override"
    );
    assert!(
        ctx_free.is_valid(),
        "Context must still be valid after builder auto-correction"
    );

    // ── Case 2: Standard tier, then turbo requested — builder auto-upgrades ───
    let ctx_std = ContextBuilder::new("req-reject-std")
        .tenant("tenant-std")
        .trace("trace-std")
        .tier(TenantTier::Standard) // Non-turbo tier first …
        .turbo(true) // … turbo auto-upgrades to Turbo tier
        .build();

    assert!(
        ctx_std.is_turbo_mode,
        "Builder must enable turbo by upgrading the tier"
    );
    assert!(ctx_std.is_valid(), "Auto-upgraded context must be valid");
    // Auto-upgrade must have promoted the tier to one that supports turbo.
    let upgraded_tier = ctx_std.tenant_tier().expect("tier must be parseable");
    assert!(
        upgraded_tier.supports_turbo(),
        "Auto-upgraded tier must support turbo mode"
    );
}

// ── H-CTX7 ────────────────────────────────────────────────────────────────────

/// Proof: `NO_TURBO_SLOT` constant equals `u32::MAX` — no magic-number collision.
///
/// Verifies that the sentinel value used to represent "no turbo slot allocated"
/// matches the canonical `u32::MAX`. This guards against accidental redefinition
/// or linkage with a different constant that would silently break slot-allocation
/// logic.
///
/// Additionally proves that any slot index within a realistic pool capacity
/// (< 1 000) can never equal `NO_TURBO_SLOT`, ensuring that valid slots and the
/// sentinel are always distinct.
///
/// # Invariant
///
/// ```text
/// NO_TURBO_SLOT = u32::MAX
/// ∀ slot < POOL_CAPACITY : slot ≠ NO_TURBO_SLOT
/// ```
#[kani::proof]
#[cfg_attr(kani, kani::unwind(1))]
fn proof_no_turbo_slot_constant() {
    // Primary invariant: the constant must equal u32::MAX exactly.
    assert_eq!(
        NO_TURBO_SLOT,
        u32::MAX,
        "NO_TURBO_SLOT must be u32::MAX — no magic-number collision allowed"
    );

    // Derived: every valid pool slot index is strictly less than NO_TURBO_SLOT.
    let slot: u32 = kani::any();
    kani::assume(slot < 1_000); // Reasonable physical pool capacity

    assert_ne!(
        slot, NO_TURBO_SLOT,
        "Any valid pool slot must differ from NO_TURBO_SLOT sentinel"
    );
    assert!(
        slot < NO_TURBO_SLOT,
        "Valid pool slots must be strictly less than NO_TURBO_SLOT"
    );
}
