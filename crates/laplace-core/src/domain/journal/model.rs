// SPDX-License-Identifier: Apache-2.0
//! Domain Model: Transaction Log
//!
//! Complete audit trail for tenant operations including execution metrics
//! and zero-copy acceleration tracking. This structure bridges standard FFI
//! and Turbo execution paths through unified logging.

use super::status::LogStatus;
use serde::{Deserialize, Serialize};

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Transaction Log Entity
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Immutable audit trail entry for tenant operation execution
///
/// This entity provides a complete record of operation lifecycle, from submission
/// through completion, with comprehensive tracking of both standard FFI and
/// zero-copy Turbo accelerated execution paths.
///
/// # Architecture
///
/// TransactionLog bridges two execution models:
/// - **Standard FFI**: Protobuf serialization with ~41.5µs context sync
/// - **Turbo Acceleration**: Shared memory zero-copy with <500ns context sync
///
/// The distinction is tracked via the `is_turbo` flag and optional slot metadata.
///
/// # Spec Compliance
///
/// - Sovereign-002: Transaction audit trail and execution tracking
/// - Performance: Zero-copy metadata collection for Turbo operations
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "20_Core_Journal",
        link = "LEP-0008-laplace-core-journal_dual_validation"
    )
)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionLog {
    /// Unique request identifier for distributed tracing
    pub request_id: String,

    /// Tenant identifier for isolation and accounting
    pub tenant_id: String,

    /// Operation name (e.g., "execute_script", "op_increment_stats")
    pub op_name: String,

    /// Execution status from lifecycle state machine
    pub status: LogStatus,

    /// Timestamp when operation was submitted (milliseconds since UNIX epoch)
    pub timestamp: i64,

    /// Duration of execution in microseconds
    ///
    /// Represents the wall-clock time from start to completion.
    /// For Turbo executions, this includes the zero-copy context sync overhead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_us: Option<u64>,

    /// Zero-copy acceleration flag
    ///
    /// `true` indicates this operation used Turbo shared memory acceleration.
    /// `false` indicates this operation used standard Protobuf FFI path.
    ///
    /// # Performance Implications
    ///
    /// Turbo executions target <500ns context sync latency, while standard
    /// FFI executions typically observe ~41.5µs for Protobuf serialization overhead.
    pub is_turbo: bool,

    /// Physical slot index in shared memory pool
    ///
    /// Only populated when `is_turbo = true`. Identifies which slot
    /// in the Turbo pool was allocated for this operation.
    ///
    /// # Use Cases
    ///
    /// - Identifying hot slots for contention analysis
    /// - Correlating with memory corruption diagnostics
    /// - Tracking slot lifecycle and reuse patterns
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turbo_slot_index: Option<usize>,

    /// Byte offset into shared memory region
    ///
    /// Only populated when `is_turbo = true`. Points to the location
    /// where this operation's context resides in shared memory.
    ///
    /// # Use Cases
    ///
    /// - Direct memory debugging and inspection
    /// - Cache line alignment verification
    /// - Memory leak detection and profiling
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turbo_memory_offset: Option<usize>,

    /// Optional error message for failed operations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,

    /// Optional error code from domain error types
    ///
    /// Maps to Spec-008 error code range for SDK propagation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<i32>,
}

impl TransactionLog {
    /// Create new transaction log entry for standard FFI execution
    ///
    /// # Arguments
    ///
    /// * `request_id` - Unique request identifier for tracing
    /// * `tenant_id` - Tenant identifier for isolation
    /// * `op_name` - Operation name for classification
    /// * `status` - Initial execution status
    ///
    /// # Returns
    ///
    /// New transaction log with current timestamp and standard FFI configuration
    pub fn new(request_id: String, tenant_id: String, op_name: String, status: LogStatus) -> Self {
        Self {
            request_id,
            tenant_id,
            op_name,
            status,
            timestamp: crate::domain::now_ms(),
            duration_us: None,
            is_turbo: false,
            turbo_slot_index: None,
            turbo_memory_offset: None,
            error_message: None,
            error_code: None,
        }
    }

    /// Create new transaction log entry for Turbo-accelerated execution
    ///
    /// This variant is used for zero-copy shared memory operations and
    /// captures the allocation metadata necessary for performance analysis.
    ///
    /// # Arguments
    ///
    /// * `request_id` - Unique request identifier for tracing
    /// * `tenant_id` - Tenant identifier for isolation
    /// * `op_name` - Operation name for classification
    /// * `status` - Initial execution status
    /// * `slot_index` - Physical slot index in shared memory pool
    /// * `memory_offset` - Byte offset in shared memory region
    ///
    /// # Returns
    ///
    /// New transaction log with Turbo metadata and current timestamp
    pub fn new_turbo(
        request_id: String,
        tenant_id: String,
        op_name: String,
        status: LogStatus,
        slot_index: usize,
        memory_offset: usize,
    ) -> Self {
        Self {
            request_id,
            tenant_id,
            op_name,
            status,
            timestamp: crate::domain::now_ms(),
            duration_us: None,
            is_turbo: true,
            turbo_slot_index: Some(slot_index),
            turbo_memory_offset: Some(memory_offset),
            error_message: None,
            error_code: None,
        }
    }

    /// Set execution duration for completed operation
    ///
    /// # Arguments
    ///
    /// * `duration_us` - Wall-clock duration in microseconds
    ///
    /// # Returns
    ///
    /// Self for method chaining
    pub fn with_duration(mut self, duration_us: u64) -> Self {
        self.duration_us = Some(duration_us);
        self
    }

    /// Set error information for failed operation
    ///
    /// # Arguments
    ///
    /// * `message` - Human-readable error description
    /// * `code` - Optional numeric error code (Spec-008 compliant)
    ///
    /// # Returns
    ///
    /// Self for method chaining
    pub fn with_error(mut self, message: String, code: Option<i32>) -> Self {
        self.error_message = Some(message);
        self.error_code = code;
        self
    }

    /// Check if operation used Turbo acceleration
    ///
    /// # Returns
    ///
    /// `true` if shared memory zero-copy path was used
    #[inline]
    pub fn is_turbo_execution(&self) -> bool {
        self.is_turbo
    }

    /// Extract Turbo slot metadata if available
    ///
    /// # Returns
    ///
    /// `Some((slot_index, memory_offset))` if Turbo execution, `None` otherwise
    pub fn turbo_slot_info(&self) -> Option<(usize, usize)> {
        match (self.turbo_slot_index, self.turbo_memory_offset) {
            (Some(idx), Some(offset)) => Some((idx, offset)),
            _ => None,
        }
    }

    /// Check if operation reached successful completion
    ///
    /// # Returns
    ///
    /// `true` only if status is Success
    #[inline]
    pub fn is_success(&self) -> bool {
        self.status.is_success()
    }

    /// Check if operation encountered failure or exception
    ///
    /// # Returns
    ///
    /// `true` if status indicates any form of failure
    #[inline]
    pub fn is_failure(&self) -> bool {
        self.status.is_failure()
    }

    /// Get execution duration in microseconds
    ///
    /// # Returns
    ///
    /// `Some(duration)` if operation has completed, `None` if still in progress
    pub fn duration_us(&self) -> Option<u64> {
        self.duration_us
    }

    /// Get execution duration in nanoseconds
    ///
    /// # Returns
    ///
    /// `Some(duration)` if available, `None` if operation in progress
    pub fn duration_ns(&self) -> Option<u64> {
        self.duration_us.map(|us| us * 1_000)
    }

    /// Classify execution latency for monitoring and alerting
    ///
    /// Categorizes latency into buckets relevant to the execution path:
    /// - Turbo operations target sub-microsecond latency
    /// - Standard FFI typically observes medium (10-100µs) latency
    ///
    /// # Returns
    ///
    /// Latency category as static string:
    /// - `"sub-microsecond"` if <1µs (Turbo target)
    /// - `"low"` if 1-10µs
    /// - `"medium"` if 10-100µs
    /// - `"high"` if >100µs
    /// - `"unknown"` if operation still in progress
    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(
            layer = "20_Core_Journal",
            link = "LEP-0008-laplace-core-journal_dual_validation"
        )
    )]
    pub fn latency_category(&self) -> &'static str {
        match self.duration_us {
            None => "unknown",
            Some(0) => "sub-microsecond",
            Some(us) if us < 10 => "low",
            Some(us) if us < 100 => "medium",
            _ => "high",
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_standard_transaction_log() {
        let log = TransactionLog::new(
            "req-123".into(),
            "tenant-abc".into(),
            "execute_script".into(),
            LogStatus::Success,
        );

        assert_eq!(log.request_id, "req-123");
        assert_eq!(log.tenant_id, "tenant-abc");
        assert_eq!(log.op_name, "execute_script");
        assert_eq!(log.status, LogStatus::Success);
        assert!(!log.is_turbo);
        assert!(log.turbo_slot_index.is_none());
        assert!(log.turbo_memory_offset.is_none());
    }

    #[test]
    fn test_turbo_transaction_log() {
        let log = TransactionLog::new_turbo(
            "req-456".into(),
            "tenant-xyz".into(),
            "execute_script".into(),
            LogStatus::Success,
            42,
            8192,
        );

        assert_eq!(log.request_id, "req-456");
        assert!(log.is_turbo);
        assert_eq!(log.turbo_slot_index, Some(42));
        assert_eq!(log.turbo_memory_offset, Some(8192));
        assert!(log.is_turbo_execution());
    }

    #[test]
    fn test_turbo_slot_info() {
        let mut log = TransactionLog::new(
            "req".into(),
            "tenant".into(),
            "op".into(),
            LogStatus::Running,
        );

        assert_eq!(log.turbo_slot_info(), None);

        log.is_turbo = true;
        log.turbo_slot_index = Some(10);
        log.turbo_memory_offset = Some(4096);

        assert_eq!(log.turbo_slot_info(), Some((10, 4096)));
    }

    #[test]
    fn test_with_duration() {
        let log = TransactionLog::new(
            "req".into(),
            "tenant".into(),
            "op".into(),
            LogStatus::Success,
        )
        .with_duration(250);

        assert_eq!(log.duration_us(), Some(250));
        assert_eq!(log.duration_ns(), Some(250_000));
    }

    #[test]
    fn test_with_error() {
        let log = TransactionLog::new(
            "req".into(),
            "tenant".into(),
            "op".into(),
            LogStatus::Failed,
        )
        .with_error("Runtime error".into(), Some(2004));

        assert_eq!(log.error_message, Some("Runtime error".into()));
        assert_eq!(log.error_code, Some(2004));
    }

    #[test]
    fn test_status_checks() {
        let success_log = TransactionLog::new(
            "req".into(),
            "tenant".into(),
            "op".into(),
            LogStatus::Success,
        );
        assert!(success_log.is_success());
        assert!(!success_log.is_failure());

        let failed_log = TransactionLog::new(
            "req".into(),
            "tenant".into(),
            "op".into(),
            LogStatus::Failed,
        );
        assert!(!failed_log.is_success());
        assert!(failed_log.is_failure());
    }

    #[test]
    fn test_latency_categorization() {
        let log1 = TransactionLog::new("r".into(), "t".into(), "o".into(), LogStatus::Success)
            .with_duration(0);
        assert_eq!(log1.latency_category(), "sub-microsecond");

        let log2 = TransactionLog::new("r".into(), "t".into(), "o".into(), LogStatus::Success)
            .with_duration(5);
        assert_eq!(log2.latency_category(), "low");

        let log3 = TransactionLog::new("r".into(), "t".into(), "o".into(), LogStatus::Success)
            .with_duration(50);
        assert_eq!(log3.latency_category(), "medium");

        let log4 = TransactionLog::new("r".into(), "t".into(), "o".into(), LogStatus::Success)
            .with_duration(200);
        assert_eq!(log4.latency_category(), "high");

        let log5 = TransactionLog::new("r".into(), "t".into(), "o".into(), LogStatus::Running);
        assert_eq!(log5.latency_category(), "unknown");
    }

    #[test]
    fn test_serialization() {
        let log = TransactionLog::new_turbo(
            "req-789".into(),
            "tenant-test".into(),
            "op_test".into(),
            LogStatus::Success,
            5,
            1024,
        )
        .with_duration(350);

        let json = serde_json::to_string(&log).unwrap();
        assert!(json.contains("req-789"));
        assert!(json.contains("turbo_slot_index"));
        assert!(json.contains("350"));

        let deserialized: TransactionLog = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.request_id, "req-789");
        assert!(deserialized.is_turbo);
        assert_eq!(deserialized.turbo_slot_index, Some(5));
        assert_eq!(deserialized.duration_us, Some(350));
    }

    #[test]
    fn test_optional_fields_serialization() {
        let log = TransactionLog::new(
            "req".into(),
            "tenant".into(),
            "op".into(),
            LogStatus::Success,
        );

        let json = serde_json::to_string(&log).unwrap();
        assert!(!json.contains("turbo_slot_index"));
        assert!(!json.contains("turbo_memory_offset"));
        assert!(!json.contains("error_message"));
    }

    #[test]
    fn test_turbo_vs_standard_latency() {
        let standard = TransactionLog::new(
            "req-std".into(),
            "tenant".into(),
            "op".into(),
            LogStatus::Success,
        )
        .with_duration(41);

        assert!(!standard.is_turbo_execution());
        assert_eq!(standard.latency_category(), "medium");

        let turbo = TransactionLog::new_turbo(
            "req-turbo".into(),
            "tenant".into(),
            "op".into(),
            LogStatus::Success,
            0,
            0,
        )
        .with_duration(0);

        assert!(turbo.is_turbo_execution());
        assert_eq!(turbo.latency_category(), "sub-microsecond");

        let speedup =
            standard.duration_us().unwrap() as f64 / (turbo.duration_us().unwrap() + 1) as f64;
        assert!(speedup > 40.0);
    }
}
