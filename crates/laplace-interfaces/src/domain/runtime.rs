//! # Runtime Domain
//!
//! Defines the contract for execution runtimes (isolate pools, WASM engines, etc.)
//! that the Laplace kernel manages for executing user workloads with resource limits
//! and lifecycle guarantees.
//!
//! The `SovereignRuntime` trait establishes a stable interface that all runtime
//! implementations must satisfy, enabling the kernel to remain independent of
//! specific runtime technologies (V8, Deno, WASM, etc.).

use crate::domain::context::SovereignContext;
use crate::error::LaplaceResult;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Runtime statistics snapshot
///
/// Provides a real-time view of runtime resource consumption and workload metrics.
/// All counters are atomic snapshots suitable for monitoring dashboards and
/// resource decision-making without blocking operations.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Runtime",
        link = "LEP-0009-laplace-interfaces-kernel_runtime_contracts"
    )
)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeStats {
    /// Total isolates available in the pool
    pub isolate_count: u32,

    /// Currently active isolates executing workloads
    pub active_isolates: u32,

    /// Current heap memory usage in bytes
    pub heap_bytes: u64,

    /// Total number of requests successfully processed since runtime start
    pub total_requests: u64,

    /// Number of requests currently queued or in-flight
    pub pending_requests: u32,

    /// Cumulative execution time across all requests in microseconds
    pub total_exec_us: u64,
}

impl RuntimeStats {
    /// Create empty stats
    pub fn new() -> Self {
        Self {
            isolate_count: 0,
            active_isolates: 0,
            heap_bytes: 0,
            total_requests: 0,
            pending_requests: 0,
            total_exec_us: 0,
        }
    }

    /// Calculate average execution time
    pub fn avg_exec_us(&self) -> f64 {
        if self.total_requests == 0 {
            0.0
        } else {
            self.total_exec_us as f64 / self.total_requests as f64
        }
    }
}

impl Default for RuntimeStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Sovereign Runtime lifecycle contract
///
/// Defines the contract that all Laplace runtimes (V8 isolate pools, WASM runtimes,
/// Deno environments, etc.) must implement for the kernel to manage execution
/// with deterministic behavior and resource constraints.
///
/// All methods use `Pin<Box<dyn Future>>` for object safety, enabling runtime
/// implementations to be selected dynamically at runtime without sacrificing
/// flexibility or performance.
///
/// # Lifecycle
///
/// 1. `init()` - Called once per runtime instance to set up pools and constraints
/// 2. `execute()` - Called for each workload execution (can be called repeatedly)
/// 3. `terminate()` - Called when the runtime should shut down gracefully
///
/// # Error Semantics
///
/// All operations return `LaplaceResult<T>` for unified error handling. Errors
/// must be mapped to standard Laplace error codes to enable consistent kernel
/// error recovery strategies.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Runtime",
        link = "LEP-0009-laplace-interfaces-kernel_runtime_contracts"
    )
)]
pub trait SovereignRuntime: Send + Sync {
    /// Initialize runtime with given context
    ///
    /// Called once per runtime instance to set up execution environment, isolate
    /// pools, memory limits, and internal state tracking. This is the primary
    /// resource allocation point; failure here means the runtime cannot be used.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The sovereign context containing configuration and isolation metadata
    ///
    /// # Returns
    ///
    /// `Ok(())` if initialization succeeds, or `LaplaceError` on failure.
    ///
    /// # Errors
    ///
    /// - `OutOfMemory`: Cannot allocate isolate pool
    /// - `InvalidRequest`: Context configuration is invalid
    /// - `Internal`: Unexpected error during initialization
    fn init(
        &self,
        ctx: &SovereignContext,
    ) -> Pin<Box<dyn Future<Output = LaplaceResult<()>> + Send + '_>>;

    /// Execute workload in the runtime
    ///
    /// Receives serialized code/request payload, executes it in a managed isolate
    /// within the established resource constraints, and returns the serialized result.
    /// Multiple concurrent `execute()` calls are supported; the runtime manages
    /// isolate pools and scheduling transparently.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The sovereign context (may differ per execution for multi-tenant scenarios)
    /// * `payload` - Serialized workload (code, arguments, configuration)
    ///
    /// # Returns
    ///
    /// `Ok(result)` with serialized output, or `LaplaceError` on execution failure.
    ///
    /// # Errors
    ///
    /// - `OutOfMemory`: Execution exceeded memory quota
    /// - `Timeout`: Execution exceeded time limit
    /// - `ExecutionError`: User code threw an exception
    /// - `InvalidRequest`: Payload is malformed or undeserializable
    /// - `Internal`: Runtime internal error
    fn execute(
        &self,
        ctx: &SovereignContext,
        payload: &[u8],
    ) -> Pin<Box<dyn Future<Output = LaplaceResult<Vec<u8>>> + Send + '_>>;

    /// Terminate runtime gracefully
    ///
    /// Called when the runtime should shut down. Must clean up all resources
    /// (isolates, memory pools, background tasks), drain pending operations,
    /// and perform graceful shutdown. This is the inverse of `init()`.
    ///
    /// Calling `terminate()` on an already-terminated runtime should be idempotent
    /// (return success without side effects).
    ///
    /// # Arguments
    ///
    /// * `ctx` - The sovereign context (used for logging and cleanup tracking)
    ///
    /// # Returns
    ///
    /// `Ok(())` if shutdown succeeds, or `LaplaceError` if critical cleanup failed.
    ///
    /// # Errors
    ///
    /// - `Timeout`: Graceful shutdown exceeded time limit; some resources may not be cleaned
    /// - `Internal`: Unexpected error during cleanup
    fn terminate(
        &self,
        ctx: &SovereignContext,
    ) -> Pin<Box<dyn Future<Output = LaplaceResult<()>> + Send + '_>>;

    /// Check if runtime is ready to accept new workloads
    ///
    /// Returns `true` if the runtime has been initialized and is not shutting down.
    /// This is a non-blocking, synchronous check suitable for fast-path decisions.
    ///
    /// # Returns
    ///
    /// `true` if ready, `false` if not initialized or terminated.
    fn is_ready(&self) -> bool;

    /// Get current resource usage snapshot
    ///
    /// Returns a snapshot of performance metrics without blocking. All counter
    /// values are atomic reads; the snapshot may reflect partially committed updates
    /// but is never inconsistent in a way that causes arithmetic errors.
    ///
    /// This method is non-blocking and safe to call from any context.
    ///
    /// # Returns
    ///
    /// Current runtime statistics.
    fn get_stats(&self) -> RuntimeStats;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_stats_creation() {
        let stats = RuntimeStats::new();
        assert_eq!(stats.isolate_count, 0);
        assert_eq!(stats.total_requests, 0);
        assert_eq!(stats.active_isolates, 0);
    }

    #[test]
    fn runtime_stats_averaging() {
        let mut stats = RuntimeStats::new();
        stats.total_requests = 100;
        stats.total_exec_us = 500_000;
        assert_eq!(stats.avg_exec_us(), 5000.0);
    }

    #[test]
    fn runtime_stats_averaging_zero_requests() {
        let stats = RuntimeStats::new();
        assert_eq!(stats.avg_exec_us(), 0.0);
    }

    #[test]
    fn runtime_stats_default() {
        let stats = RuntimeStats::default();
        assert_eq!(stats.isolate_count, 0);
        assert_eq!(stats.total_requests, 0);
        assert_eq!(stats.heap_bytes, 0);
    }

    #[test]
    fn runtime_stats_serialization() {
        let stats = RuntimeStats {
            isolate_count: 5,
            active_isolates: 2,
            heap_bytes: 1024 * 1024,
            total_requests: 1000,
            pending_requests: 3,
            total_exec_us: 50_000_000,
        };

        // Verify that stats can be serialized (trait bounds are correct)
        let _json = serde_json::to_string(&stats);
    }
}
