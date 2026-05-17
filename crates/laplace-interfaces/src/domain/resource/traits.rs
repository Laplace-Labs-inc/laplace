//! Resource tracking trait contracts

use super::types::{RequestResult, ResourceError, ResourceId, ResourceType, ThreadId};
use crate::domain::SovereignContext;
use crate::error::LaplaceResult;
use std::fmt;
use std::future::Future;
use std::pin::Pin;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Resource tracking interface (contract for production and verification backends)
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Resource",
        link = "LEP-0004-laplace-interfaces-resource_domain_contracts"
    )
)]
pub trait ResourceTracker: Send + Sync + fmt::Debug {
    /// Create a new resource tracker instance
    fn new(num_threads: usize, num_resources: usize) -> Self;

    /// Request a resource
    fn request(
        &mut self,
        thread: ThreadId,
        resource: ResourceId,
    ) -> Result<RequestResult, ResourceError>;

    /// Release a resource
    fn release(&mut self, thread: ThreadId, resource: ResourceId) -> Result<(), ResourceError>;

    /// Mark a thread as finished
    fn on_finish(&mut self, thread: ThreadId) -> Result<(), ResourceError>;

    /// Check whether a deadlock exists
    fn has_deadlock(&self) -> bool;

    /// Get the set of threads involved in a deadlock
    fn deadlocked_threads(&self) -> Vec<ThreadId>;

    /// Get contention score for Ki-DPOR heuristic
    fn contention_score(&self) -> u32;

    /// Get interleaving score for Ki-DPOR heuristic
    fn interleaving_score(&self) -> u32;
}

/// Resource Guard: quota and limit enforcement
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Resource",
        link = "LEP-0004-laplace-interfaces-resource_domain_contracts"
    )
)]
pub trait ResourceGuard: Send + Sync {
    /// Check if operation would violate resource limits
    fn check_limit(&self, ctx: &SovereignContext, resource: &ResourceType) -> LaplaceResult<()>;

    /// Record resource usage after operation
    fn record_usage(
        &self,
        ctx: &SovereignContext,
        resource: &ResourceType,
        amount: u64,
    ) -> Pin<Box<dyn Future<Output = LaplaceResult<()>> + Send + '_>>;

    /// Reset quota for a tenant
    fn reset_quota(
        &self,
        tenant_id: &str,
    ) -> Pin<Box<dyn Future<Output = LaplaceResult<()>> + Send + '_>>;

    /// Get current usage for a tenant
    fn get_usage(&self, tenant_id: &str) -> ResourceUsage;
}

/// Current resource usage snapshot for a single tenant, returned by [`ResourceGuard::get_usage`].
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "20_Core_Resource",
        link = "LEP-0007-laplace-core-resource_sovereignty"
    )
)]
#[derive(Debug, Clone)]
pub struct ResourceUsage {
    /// Unique tenant identifier this snapshot belongs to.
    pub tenant_id: String,
    /// Accumulated CPU time consumed this period, in microseconds.
    pub cpu_used_us: u64,
    /// Heap memory currently allocated, in bytes.
    pub memory_used_bytes: u64,
    /// Network bandwidth consumed this period, in bytes.
    pub network_used_bytes: u64,
    /// Number of requests currently in flight.
    pub concurrent_requests: u32,
    /// Persistent storage currently in use, in bytes.
    pub storage_used_bytes: u64,
}

impl ResourceUsage {
    /// Creates a zeroed usage snapshot for the given tenant.
    ///
    /// - **Arguments:** `tenant_id` — identifier of the tenant; converted to `String`.
    /// - **Returns:** A [`ResourceUsage`] with all counters set to zero.
    /// - **Ownership:** `tenant_id` is consumed (moved/converted via `Into<String>`).
    pub fn new(tenant_id: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            cpu_used_us: 0,
            memory_used_bytes: 0,
            network_used_bytes: 0,
            concurrent_requests: 0,
            storage_used_bytes: 0,
        }
    }

    /// Returns `true` when every counter is within the Free-tier quota.
    ///
    /// - **Returns:** `true` if all current usage values are at or below [`ResourceType::default_limit_free`].
    /// - **Ownership:** `self` is immutably borrowed.
    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(
            layer = "20_Core_Resource",
            link = "LEP-0007-laplace-core-resource_sovereignty"
        )
    )]
    pub fn is_within_free_tier(&self) -> bool {
        self.cpu_used_us <= ResourceType::CpuMicroseconds.default_limit_free()
            && self.memory_used_bytes <= ResourceType::MemoryBytes.default_limit_free()
            && self.network_used_bytes <= ResourceType::NetworkBytes.default_limit_free()
            && self.concurrent_requests as u64
                <= ResourceType::ConcurrentRequests.default_limit_free()
    }
}

impl Default for ResourceUsage {
    fn default() -> Self {
        Self::new("unknown")
    }
}
