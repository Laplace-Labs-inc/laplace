//! Tenant Domain Layer
//!
//! Migrated business logic from laplace-axiom to laplace-core (Phase R2: Brain Migration).
//!
//! This module provides extension traits for types defined in laplace-interfaces,
//! enabling implementation without violating Rust's orphan rule.
//!
//! # Architecture
//!
//! - **Extension Traits**: `TenantTierExt`, `ResourceConfigExt`, `TenantMetadataExt`, `TenantErrorExt`
//! - **Policy Structs**: `PathPolicy`, `ResourcePolicy`, `TierRecommendationPolicy`
//!
//! # Zero-Copy Readiness
//!
//! The tenant module distinguishes between:
//! - **Standard FFI**: Protobuf serialization (~41.5µs context sync)
//! - **Turbo Acceleration**: Shared memory zero-copy (<500ns context sync)
//!
//! Tier qualification for Turbo is determined at domain level via
//! `TenantTierExt::uses_turbo_acceleration()`.

pub mod model;
pub mod policy;
pub mod tier;

#[cfg(kani)]
mod proofs;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Public Re-exports
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// Extension traits for types from laplace-interfaces
pub use model::{ResourceConfigExt, TenantMetadataExt};
pub use tier::TenantTierExt;

// Policy structs (stateless, pure business logic)
pub use policy::{PathPolicy, ResourcePolicy, TierRecommendationPolicy};

// Re-export types from laplace-interfaces for convenience
pub use laplace_interfaces::domain::{ResourceConfig, TenantMetadata, TenantTier};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Time Utilities
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Get current time in milliseconds since UNIX epoch
///
/// # Returns
///
/// Current timestamp as i64 milliseconds
#[inline]
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Get current time in microseconds since UNIX epoch
///
/// # Returns
///
/// Current timestamp as i64 microseconds
#[inline]
pub fn now_us() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64
}

/// Get current time in nanoseconds since UNIX epoch
///
/// # Returns
///
/// Current timestamp as i64 nanoseconds (may overflow for far-future dates)
#[inline]
pub fn now_ns() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as i64
}
