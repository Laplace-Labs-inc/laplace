//! # FFI-Compatible Error Codes
//!
//! Unified error codes for the Rust-TypeScript FFI boundary.
//! Sovereign Bridge ABI v1.1.0 - Error Propagation.
//!
//! Defines the authoritative error codes that flow across the Rust-TypeScript FFI boundary.
//! All error codes are FFI-compatible (u32) and compatible with Protobuf serialization.
//!
//! This module serves as the Single Source of Truth (SSOT) for all Laplace error definitions.
//! The Protobuf error.proto file and TypeScript FFI bindings derive their error codes from
//! this Rust enumeration.

use std::fmt;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Laplace error codes for FFI boundary.
///
/// Represents the complete error taxonomy of the Laplace system. All error codes
/// are organized by severity and logical domain to enable client-side error handling
/// and recovery strategies.
///
/// The `#[repr(u32)]` attribute ensures compatibility with C FFI conventions and
/// Protobuf encoding. Each error code carries semantic meaning that determines
/// whether a client should retry, escalate, or fail the request.
///
/// # Error Code Ranges
///
/// - **0x0000**: Success (0)
/// - **0x1000-0x1FFF**: Internal errors (ABI, memory, context)
/// - **0x2000-0x2FFF**: Timeout errors (operation deadlines)
/// - **0x3000-0x3FFF**: Resource errors (quotas, memory, concurrency)
/// - **0x4000-0x4FFF**: Validation errors (handshake, authorization)
/// - **0x5000-0x5FFF**: Network errors (communication failures)
/// - **0x6000-0x6FFF**: Domain-specific errors (verification, scheduling)
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Error",
        link = "LEP-0017-laplace-interfaces-error_taxonomy"
    )
)]
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaplaceError {
    // ═══════════════════════════════════════════════════════════════════════════════
    // Success (0x0000)
    // ═══════════════════════════════════════════════════════════════════════════════
    /// Successful operation (0).
    ///
    /// Returned when the operation completed without error.
    /// No client action required.
    Success = 0,

    // ═══════════════════════════════════════════════════════════════════════════════
    // Internal Errors (0x1000-0x1FFF)
    // ═══════════════════════════════════════════════════════════════════════════════
    // Errors in this range indicate internal system failures that prevent operation
    // completion. These are not due to client action and generally cannot be retried
    // without external intervention (e.g., system restart, bug fix).
    /// Generic internal error (1000).
    ///
    /// Returned for unexpected panics, unrecoverable state, or other internal
    /// system failures. The error message and logs should be examined for details.
    /// Recommendation: Do not retry; escalate to operations team.
    Internal = 1000,

    /// Context is missing or invalid (1001).
    ///
    /// The SovereignContext passed to the kernel is null, malformed, or fails
    /// validation (empty identifiers, out-of-range priority/tier).
    /// Recommendation: Verify context construction and validation.
    InvalidContext = 1001,

    /// ABI version mismatch detected (1002).
    ///
    /// The client's FFI ABI version does not match the kernel's version.
    /// This indicates incompatible library versions across the Rust-Deno boundary.
    /// Recommendation: Update libraries to compatible versions.
    AbiMismatch = 1002,

    /// Memory layout violation or alignment error (1003).
    ///
    /// FFI memory layout is misaligned or does not match expected structure.
    /// Typically indicates incorrect pointer arithmetic or struct size mismatches.
    /// Recommendation: Verify FFI memory layout and struct definitions.
    MemoryAlignment = 1003,

    /// Pointer validation failed (1004).
    ///
    /// A pointer passed through FFI is null, out of bounds, or otherwise invalid.
    /// The kernel detected unsafe memory access that would cause undefined behavior.
    /// Recommendation: Validate all pointers before FFI calls.
    InvalidPointer = 1004,

    // ═══════════════════════════════════════════════════════════════════════════════
    // Timeout Errors (0x2000-0x2FFF)
    // ═══════════════════════════════════════════════════════════════════════════════
    // Errors in this range indicate that operations exceeded their time limits.
    // All timeout errors are retryable with exponential backoff and longer deadlines.
    /// Operation exceeded deadline (2000).
    ///
    /// Generic timeout error when operation deadline is exceeded.
    /// The kernel's watchdog timer triggered, indicating the operation took too long.
    /// Recommendation: Retry with exponential backoff and longer deadline.
    Timeout = 2000,

    /// Kernel operation timeout (2001).
    ///
    /// The kernel-side operation (e.g., scheduler, verification, isolation)
    /// exceeded its time budget.
    /// Recommendation: Retry with longer timeout, consider batch operations.
    KernelTimeout = 2001,

    /// SDK operation timeout (2002).
    ///
    /// The SDK-side operation (e.g., serialization, FFI marshaling)
    /// exceeded its time budget.
    /// Recommendation: Retry, consider optimizing SDK code.
    SdkTimeout = 2002,

    /// Lock acquisition timeout (2003).
    ///
    /// The kernel could not acquire a required lock within the time limit.
    /// Indicates contention on shared resources.
    /// Recommendation: Retry with exponential backoff; consider spreading load.
    LockTimeout = 2003,

    // ═══════════════════════════════════════════════════════════════════════════════
    // Resource Errors (0x3000-0x3FFF)
    // ═══════════════════════════════════════════════════════════════════════════════
    // Errors in this range indicate resource exhaustion or quota violations.
    // These errors may be retryable if resources are freed, or may require
    // scaling or SLA upgrade.
    /// Resource quota exceeded (3000).
    ///
    /// The tenant's resource quota (CPU, memory, concurrency, or other quota)
    /// has been exceeded. The kernel's Resource Guard rejected the request.
    /// Recommendation: Wait for quota reset, or upgrade tenant tier.
    QuotaExceeded = 3000,

    /// Memory allocation failed (3001).
    ///
    /// The kernel could not allocate the required memory for operation.
    /// May indicate heap pressure or a memory leak.
    /// Recommendation: Retry after garbage collection; escalate if persistent.
    OutOfMemory = 3001,

    /// CPU credit exhausted (3002).
    ///
    /// The tenant's CPU credit budget has been exhausted for the billing period.
    /// Further operations are rate-limited until the budget resets.
    /// Recommendation: Wait for credit reset, or upgrade tenant tier.
    CpuQuotaExceeded = 3002,

    /// Concurrent request limit reached (3003).
    ///
    /// The maximum number of concurrent requests for this tenant has been reached.
    /// The kernel rejected the new request to maintain SLA guarantees.
    /// Recommendation: Retry after existing requests complete; consider batching.
    ConcurrencyLimitExceeded = 3003,

    /// Connection pool exhausted (3004).
    ///
    /// The kernel's connection pool has no available connections.
    /// All connections are in use or pending cleanup.
    /// Recommendation: Retry with backoff; the pool will refill as connections close.
    PoolExhausted = 3004,

    // ═══════════════════════════════════════════════════════════════════════════════
    // Validation Errors (0x4000-0x4FFF)
    // ═══════════════════════════════════════════════════════════════════════════════
    // Errors in this range indicate that the request failed validation or
    // authorization checks. These are generally permanent and should not be retried
    // without fixing the underlying issue.
    /// Handshake failed (4000).
    ///
    /// The FFI handshake between kernel and SDK failed. This typically indicates
    /// capability negotiation failure, ABI incompatibility, or protocol violation.
    /// Recommendation: Verify FFI setup and library versions.
    HandshakeFailed = 4000,

    /// Invalid request format or schema (4001).
    ///
    /// The request does not match the expected schema or contains invalid data.
    /// Examples: malformed Protobuf, invalid field values, missing required fields.
    /// Recommendation: Fix request format; do not retry without changes.
    InvalidRequest = 4001,

    /// Unsupported operation version (4002).
    ///
    /// The operation uses a version not supported by the kernel.
    /// May indicate outdated client or forward-incompatibility issue.
    /// Recommendation: Update client or kernel to compatible versions.
    VersionMismatch = 4002,

    /// Tenant not found or inactive (4003).
    ///
    /// The tenant_id in the context does not exist or is in an inactive state.
    /// The kernel rejected the request because the tenant is not authorized.
    /// Recommendation: Verify tenant_id; contact support if tenant should be active.
    TenantNotFound = 4003,

    /// Authorization check failed (4004).
    ///
    /// The request failed permission checks. The client does not have access to
    /// the requested resource or operation.
    /// Recommendation: Verify credentials and permissions; do not retry.
    Unauthorized = 4004,

    // ═══════════════════════════════════════════════════════════════════════════════
    // Network Errors (0x5000-0x5FFF)
    // ═══════════════════════════════════════════════════════════════════════════════
    // Errors in this range indicate communication or serialization failures.
    // Most network errors are transient and retryable with exponential backoff.
    /// Network communication failure (5000).
    ///
    /// Generic network error during communication with the kernel.
    /// May indicate packet loss, network latency, or routing issues.
    /// Recommendation: Retry with exponential backoff.
    NetworkError = 5000,

    /// Connection refused or reset (5001).
    ///
    /// The kernel connection was refused, reset, or terminated unexpectedly.
    /// May indicate the kernel is restarting or unavailable.
    /// Recommendation: Retry; the connection may be re-established.
    ConnectionFailed = 5001,

    /// Serialization/deserialization failed (5002).
    ///
    /// Protobuf or binary serialization/deserialization failed.
    /// May indicate corrupt data or version mismatch.
    /// Recommendation: Verify data format and retry.
    SerializationError = 5002,

    // ═══════════════════════════════════════════════════════════════════════════════
    // Domain-Specific Errors (0x6000-0x6FFF)
    // ═══════════════════════════════════════════════════════════════════════════════
    // Errors in this range are specific to Laplace domain operations
    // (formal verification, scheduling, isolation).
    /// Axiom/formal verification error (6000).
    ///
    /// The formal verification component (Axiom) encountered an error.
    /// May indicate unsupported operation or verification timeout.
    /// Recommendation: Check verification configuration; retry or escalate.
    VerificationError = 6000,

    /// Kernel isolate pool error (6001).
    ///
    /// The isolate pool that manages V8 runtime isolation failed.
    /// May indicate exhaustion, panic, or protocol violation.
    /// Recommendation: Retry; escalate if persistent.
    IsolatePoolError = 6001,

    /// Scheduler error (6002).
    ///
    /// The kernel scheduler (DPOR, Ki-DPOR, or round-robin) encountered an error.
    /// May indicate state corruption or algorithm failure.
    /// Recommendation: Escalate; likely indicates a bug.
    SchedulerError = 6002,
}

impl LaplaceError {
    /// Convert a u32 error code back to the enum variant.
    ///
    /// Maps unknown codes to `Internal` for safety. This ensures that new error
    /// codes added in future versions do not crash older clients.
    ///
    /// # Arguments
    ///
    /// * `code` - Error code as u32
    ///
    /// # Returns
    ///
    /// The corresponding `LaplaceError` variant, or `Internal` if code is unknown.
    pub fn from_code(code: u32) -> Self {
        match code {
            0 => LaplaceError::Success,
            1000 => LaplaceError::Internal,
            1001 => LaplaceError::InvalidContext,
            1002 => LaplaceError::AbiMismatch,
            1003 => LaplaceError::MemoryAlignment,
            1004 => LaplaceError::InvalidPointer,
            2000 => LaplaceError::Timeout,
            2001 => LaplaceError::KernelTimeout,
            2002 => LaplaceError::SdkTimeout,
            2003 => LaplaceError::LockTimeout,
            3000 => LaplaceError::QuotaExceeded,
            3001 => LaplaceError::OutOfMemory,
            3002 => LaplaceError::CpuQuotaExceeded,
            3003 => LaplaceError::ConcurrencyLimitExceeded,
            3004 => LaplaceError::PoolExhausted,
            4000 => LaplaceError::HandshakeFailed,
            4001 => LaplaceError::InvalidRequest,
            4002 => LaplaceError::VersionMismatch,
            4003 => LaplaceError::TenantNotFound,
            4004 => LaplaceError::Unauthorized,
            5000 => LaplaceError::NetworkError,
            5001 => LaplaceError::ConnectionFailed,
            5002 => LaplaceError::SerializationError,
            6000 => LaplaceError::VerificationError,
            6001 => LaplaceError::IsolatePoolError,
            6002 => LaplaceError::SchedulerError,
            _ => LaplaceError::Internal, // Unknown codes map to Internal for forward compatibility
        }
    }

    /// Get human-readable error message.
    ///
    /// Returns a static string describing the error in user-facing language.
    /// Suitable for logging, debugging, and error reporting.
    ///
    /// # Returns
    ///
    /// Static string describing the error.
    pub fn message(&self) -> &'static str {
        match self {
            LaplaceError::Success => "Success",
            LaplaceError::Internal => "Internal error",
            LaplaceError::InvalidContext => "Invalid context",
            LaplaceError::AbiMismatch => "ABI version mismatch",
            LaplaceError::MemoryAlignment => "Memory alignment violation",
            LaplaceError::InvalidPointer => "Invalid pointer",
            LaplaceError::Timeout => "Operation timeout",
            LaplaceError::KernelTimeout => "Kernel operation timeout",
            LaplaceError::SdkTimeout => "SDK operation timeout",
            LaplaceError::LockTimeout => "Lock acquisition timeout",
            LaplaceError::QuotaExceeded => "Resource quota exceeded",
            LaplaceError::OutOfMemory => "Out of memory",
            LaplaceError::CpuQuotaExceeded => "CPU quota exceeded",
            LaplaceError::ConcurrencyLimitExceeded => "Concurrency limit exceeded",
            LaplaceError::PoolExhausted => "Connection pool exhausted",
            LaplaceError::HandshakeFailed => "Handshake failed",
            LaplaceError::InvalidRequest => "Invalid request",
            LaplaceError::VersionMismatch => "Version mismatch",
            LaplaceError::TenantNotFound => "Tenant not found",
            LaplaceError::Unauthorized => "Unauthorized",
            LaplaceError::NetworkError => "Network error",
            LaplaceError::ConnectionFailed => "Connection failed",
            LaplaceError::SerializationError => "Serialization error",
            LaplaceError::VerificationError => "Verification error",
            LaplaceError::IsolatePoolError => "Isolate pool error",
            LaplaceError::SchedulerError => "Scheduler error",
        }
    }

    /// Check if this error is retryable.
    ///
    /// Retryable errors are transient conditions that may succeed if the
    /// operation is attempted again, typically with exponential backoff.
    ///
    /// # Returns
    ///
    /// `true` if the error is retryable, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// if error.is_retryable() {
    ///     // Implement exponential backoff retry
    /// } else {
    ///     // Permanent error; fail fast
    /// }
    /// ```
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            LaplaceError::Timeout
                | LaplaceError::KernelTimeout
                | LaplaceError::SdkTimeout
                | LaplaceError::LockTimeout
                | LaplaceError::NetworkError
                | LaplaceError::ConnectionFailed
                | LaplaceError::PoolExhausted
        )
    }

    /// Check if this error indicates resource exhaustion.
    ///
    /// Resource errors indicate that the kernel has hit a quota or limit.
    /// The client may choose to wait for the resource to be freed, scale
    /// the infrastructure, or upgrade the tenant tier.
    ///
    /// # Returns
    ///
    /// `true` if the error is a resource error, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// if error.is_resource_error() {
    ///     // Trigger autoscaling or quota reset logic
    /// }
    /// ```
    pub fn is_resource_error(&self) -> bool {
        matches!(
            self,
            LaplaceError::QuotaExceeded
                | LaplaceError::OutOfMemory
                | LaplaceError::CpuQuotaExceeded
                | LaplaceError::ConcurrencyLimitExceeded
                | LaplaceError::PoolExhausted
        )
    }
}

impl fmt::Display for LaplaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "LaplaceError::{:?} (0x{:04x}): {}",
            self,
            *self as u32,
            self.message()
        )
    }
}

impl std::error::Error for LaplaceError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_round_trip() {
        let err = LaplaceError::AbiMismatch;
        let code = err as u32;
        let recovered = LaplaceError::from_code(code);
        assert_eq!(err, recovered);
    }

    #[test]
    fn error_messages_non_empty() {
        for code in [
            LaplaceError::Success,
            LaplaceError::Timeout,
            LaplaceError::QuotaExceeded,
        ] {
            assert!(!code.message().is_empty());
        }
    }

    #[test]
    fn retryable_errors() {
        assert!(LaplaceError::Timeout.is_retryable());
        assert!(LaplaceError::NetworkError.is_retryable());
        assert!(!LaplaceError::Unauthorized.is_retryable());
    }

    #[test]
    fn resource_errors() {
        assert!(LaplaceError::QuotaExceeded.is_resource_error());
        assert!(LaplaceError::OutOfMemory.is_resource_error());
        assert!(!LaplaceError::Timeout.is_resource_error());
    }

    #[test]
    fn forward_compatibility() {
        let unknown = LaplaceError::from_code(9999);
        assert_eq!(unknown, LaplaceError::Internal);
    }
}
