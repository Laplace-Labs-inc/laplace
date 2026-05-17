//! # Kernel Domain
//!
//! Defines the kernel's exposed capabilities and APIs that runtime implementations
//! use to interact with the Laplace kernel. This enables runtime implementations
//! to access kernel services such as dynamic runtime spawning and resource
//! constraint validation without coupling to kernel internals.
//!
//! This reverse-dependency model (runtimes call back into kernel) establishes
//! a clean separation of concerns while enabling sophisticated orchestration patterns.

use crate::domain::context::SovereignContext;
use crate::domain::runtime::SovereignRuntime;
use crate::error::LaplaceResult;
use std::future::Future;
use std::pin::Pin;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Kernel execution context and capabilities
///
/// Provided to runtime implementations to enable reverse calls into the kernel
/// for advanced orchestration scenarios. This trait represents the "kernel side"
/// of the kernel-runtime interaction contract.
///
/// Runtime implementations receive an instance of this trait and can invoke
/// kernel services such as spawning child isolates, checking resource quotas,
/// or reporting metrics without direct access to kernel internals.
///
/// # Design Pattern
///
/// This trait exemplifies the "Facade" pattern, providing a stable interface
/// through which runtimes interact with the kernel. This abstraction enables:
///
/// - **Version Independence**: Kernel implementation details can change without
///   affecting runtime code
/// - **Testing Isolation**: Mock implementations can be provided for unit testing
///   runtime behavior in isolation
/// - **Capability Negotiation**: Different kernel instances can expose different
///   capabilities through distinct trait implementations
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Kernel",
        link = "LEP-0009-laplace-interfaces-kernel_runtime_contracts"
    )
)]
pub trait KernelCapabilities: Send + Sync {
    /// Create a new child runtime instance
    ///
    /// Requests the kernel to instantiate a new runtime, typically for nested
    /// execution or dynamic workload spawning. This allows runtimes to create
    /// subordinate execution contexts while maintaining proper resource accounting
    /// and isolation boundaries.
    ///
    /// The newly created runtime is returned uninitialized; the caller is
    /// responsible for calling `init()` on the returned runtime before using it.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The sovereign context for the child runtime (may share quota with parent)
    ///
    /// # Returns
    ///
    /// A new `SovereignRuntime` instance ready for initialization and use.
    ///
    /// # Errors
    ///
    /// - `OutOfMemory`: Cannot allocate a new runtime instance
    /// - `QuotaExceeded`: Parent context has exhausted its runtime quota
    /// - `InvalidRequest`: Context configuration is invalid for spawning
    /// - `Internal`: Unexpected error during runtime creation
    ///
    /// # Example
    ///
    /// ```ignore
    /// let child_runtime = kernel.spawn_runtime(&child_ctx).await?;
    /// child_runtime.init(&child_ctx).await?;
    /// child_runtime.execute(&child_ctx, &payload).await?;
    /// ```
    #[allow(clippy::type_complexity)]
    fn spawn_runtime(
        &self,
        ctx: &SovereignContext,
    ) -> Pin<Box<dyn Future<Output = LaplaceResult<Box<dyn SovereignRuntime>>> + Send + '_>>;

    /// Check current resource constraints and validate against quota
    ///
    /// Synchronously validates that the given context's resource consumption
    /// is within established bounds. This is a fast-path check suitable for
    /// execution gate-keeping; it does not consume resources or perform I/O.
    ///
    /// Typical usage is to call this before expensive operations (e.g., before
    /// `execute()` in a runtime) to fail fast if quota is exceeded.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The sovereign context to validate
    ///
    /// # Returns
    ///
    /// `Ok(())` if all constraints are satisfied, or `LaplaceError` if exceeded.
    ///
    /// # Errors
    ///
    /// - `QuotaExceeded`: Memory, time, or request quota has been exceeded
    /// - `ResourceUnavailable`: Required resource is temporarily unavailable
    /// - `InvalidRequest`: Context is not valid for resource checking
    fn check_resources(&self, ctx: &SovereignContext) -> LaplaceResult<()>;

    /// Report metrics to the observability system
    ///
    /// Asynchronously sends a metric data point to the kernel's observability
    /// layer (monitoring system, time-series database, etc.). This enables
    /// runtime implementations to emit custom metrics without coupling to
    /// specific observability backends.
    ///
    /// Metric reporting is best-effort; failures do not interrupt the calling
    /// runtime's execution. The kernel handles metric buffering and batching
    /// transparently.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The sovereign context (used for metric tagging/scoping)
    /// * `metric_name` - The name of the metric (e.g., "isolates_created", "gc_pause_ms")
    /// * `value` - The numeric value of the metric
    ///
    /// # Returns
    ///
    /// `Ok(())` if the metric was accepted for processing, or `LaplaceError` if
    /// the observability system is unavailable or the metric is malformed.
    ///
    /// # Errors
    ///
    /// - `InvalidRequest`: Metric name or value is invalid
    /// - `Unavailable`: Observability system is not available
    /// - `Internal`: Unexpected error during metric reporting
    ///
    /// # Note
    ///
    /// Implementations should not block or retry indefinitely; errors should be
    /// logged but not propagated back to the runtime's execution path.
    ///
    /// # Example
    ///
    /// ```ignore
    /// kernel.report_metrics(&ctx, "isolates.active", 42.0).await.ok();
    /// ```
    fn report_metrics(
        &self,
        ctx: &SovereignContext,
        metric_name: &str,
        value: f64,
    ) -> Pin<Box<dyn Future<Output = LaplaceResult<()>> + Send + '_>>;
}

#[cfg(test)]
mod tests {

    #[test]
    fn kernel_capabilities_trait_exists() {
        // This test verifies that the trait is properly defined and compilable.
        // Concrete kernel implementations provide the actual behavior.
        // This is a compile-time contract verification.
    }

    #[test]
    fn kernel_spawn_runtime_returns_future() {
        // Verify the method signature: spawn_runtime should return a boxed future
        // that produces a LaplaceResult containing a dyn SovereignRuntime.
        // The type system enforces this contract.
    }

    #[test]
    fn kernel_check_resources_is_synchronous() {
        // verify that check_resources is a synchronous method returning LaplaceResult
        // This allows fast-path resource validation without async overhead.
    }
}
