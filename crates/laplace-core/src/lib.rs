//! # Laplace Core
//!
//! Shared abstraction layer between Kernel and KNUL (Network Utility Link).
//! Defines FFI boundaries, context propagation contracts, error codes, and
//! network transport abstractions that form the foundation of the Laplace
//! formal verification and execution platform.
//!
//! ## Design Philosophy
//!
//! Laplace Core embodies the principle of separation of concerns through careful
//! abstraction boundaries. The kernel logic remains agnostic to transport
//! implementation details, allowing multiple backends (QUIC, HTTP/3, mock
//! transports for testing) without modification to kernel code.
//!
//! ## Module Organization
//!
//! The library is organized into logical layers that establish clear dependencies:
//!
//! - `domain`: Domain layer containing core business abstractions
//!   - `context`: Deterministic context propagation structures
//!   - `resource`: Resource quota enforcement and usage tracking)
//! - `infrastructure`: Low-level system abstractions

pub mod domain;
pub mod infrastructure;

// Re-export domain types at crate root for primary access pattern
pub use domain::{
    now_ms, now_ns, now_us, ContextBuilder, HealthStatus, LogStatus, PathPolicy, PoolHealthCheck,
    PoolPolicy, PoolSnapshot, ResourceConfig, ResourceConfigExt, ResourceGuard, ResourcePolicy,
    ResourceType, ResourceUsage, StorageStrategy, TenantMetadata, TenantMetadataExt, TenantTier,
    TenantTierExt, TierRecommendationPolicy, TransactionLog, DEFAULT_IDLE_TIMEOUT_SECS,
    DEFAULT_POOL_SIZE, STANDARD_LATENCY_BASELINE_NS, TURBO_LATENCY_TARGET_NS,
};

// Re-export ABI types from infrastructure layer at top level for convenience
pub use laplace_interfaces::abi::{
    FfiBuffer, FfiLockState, FfiQuicConfig, FfiResponse, SharedMemoryMetadata,
};

/// Library version following semantic versioning
pub const KREPIS_CORE_VERSION: &str = "0.1.0";

#[cfg(test)]
mod lib_tests {
    use super::*;
    use laplace_interfaces::domain::SovereignContext;

    #[test]
    fn version_constants_defined() {
        assert!(!KREPIS_CORE_VERSION.is_empty());
    }

    #[test]
    fn domain_context_accessible() {
        // Verify that context types are re-exported at crate root
        let ctx = SovereignContext::new(
            "test-req".to_string(),
            "test-tenant".to_string(),
            "test-trace".to_string(),
        );
        assert!(ctx.is_valid());

        let ctx_built = ContextBuilder::new("builder-req")
            .tenant("builder-tenant")
            .trace("builder-trace")
            .build();
        assert!(ctx_built.is_valid());
    }

    #[test]
    fn domain_resource_accessible() {
        // Verify that resource types are re-exported at crate root
        let usage = ResourceUsage::new("tenant-test");
        assert_eq!(usage.tenant_id, "tenant-test");
        assert!(usage.is_within_free_tier());

        // Verify resource types and limits are accessible
        let cpu_limit = ResourceType::CpuMicroseconds.default_limit_free();
        assert_eq!(cpu_limit, 100_000);
    }
}
