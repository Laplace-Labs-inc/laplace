//! Domain Model: Transaction Log Status
//!
//! Execution lifecycle states for tenant operations and their transitions.
//! Pure enumeration with no infrastructure dependencies.

use serde::{Deserialize, Serialize};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Log Status Enumeration
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Transaction execution lifecycle state
///
/// This enumeration tracks the complete lifecycle of a tenant operation from
/// initial submission through terminal completion. It enables comprehensive
/// auditing and performance monitoring across the execution pipeline.
///
/// # Spec Compliance
///
/// - Sovereign-002: Transaction audit trail
/// - Spec-008: Status code mapping for SDK propagation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LogStatus {
    /// Operation has been queued but not yet started
    ///
    /// This state occurs when a tenant hits their concurrency limit
    /// and must await slot availability in the execution queue.
    #[default]
    Pending,

    /// Operation is currently executing
    ///
    /// V8 isolate is active and tenant code is executing within the kernel.
    Running,

    /// Operation completed successfully
    ///
    /// All execution finished without errors and results are available.
    Success,

    /// Operation completed with errors
    ///
    /// Runtime error, constraint violation, or resource exception occurred.
    Failed,

    /// Operation was terminated by watchdog timeout
    ///
    /// Execution exceeded configured time limit and was forcibly terminated.
    Timeout,

    /// Operation was cancelled by user request
    ///
    /// User explicitly requested cancellation before completion.
    Cancelled,

    /// Isolate was evicted from pool
    ///
    /// LRU eviction occurred due to memory pressure or idle timeout.
    Evicted,

    /// Turbo acceleration pool exhaustion fallback
    ///
    /// Shared memory pool was exhausted and execution fell back to Standard FFI.
    TurboFallback,
}

impl LogStatus {
    /// Check if status represents a terminal state
    ///
    /// Terminal states indicate the operation has completed in some form.
    ///
    /// # Returns
    ///
    /// `true` if operation has reached terminal state
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Success | Self::Failed | Self::Timeout | Self::Cancelled | Self::Evicted
        )
    }

    /// Check if status represents successful completion
    ///
    /// # Returns
    ///
    /// `true` only if status is Success
    pub fn is_success(self) -> bool {
        matches!(self, Self::Success)
    }

    /// Check if status represents failure or exceptional termination
    ///
    /// # Returns
    ///
    /// `true` if operation failed, timed out, was cancelled, or was evicted
    pub fn is_failure(self) -> bool {
        matches!(
            self,
            Self::Failed | Self::Timeout | Self::Cancelled | Self::Evicted
        )
    }

    /// Check if operation is still in progress
    ///
    /// # Returns
    ///
    /// `true` if status is Pending or Running
    pub fn is_in_progress(self) -> bool {
        matches!(self, Self::Pending | Self::Running)
    }

    /// Check if status indicates Turbo-specific event
    ///
    /// # Returns
    ///
    /// `true` if status relates to Turbo acceleration mechanics
    pub fn is_turbo_related(self) -> bool {
        matches!(self, Self::TurboFallback)
    }

    /// Convert status to numeric code for protobuf serialization
    ///
    /// # Returns
    ///
    /// Status code in range 0-7
    pub fn to_code(self) -> u8 {
        match self {
            Self::Pending => 0,
            Self::Running => 1,
            Self::Success => 2,
            Self::Failed => 3,
            Self::Timeout => 4,
            Self::Cancelled => 5,
            Self::Evicted => 6,
            Self::TurboFallback => 7,
        }
    }

    /// Parse status from numeric code
    ///
    /// # Arguments
    ///
    /// * `code` - Numeric status code (0-7)
    ///
    /// # Returns
    ///
    /// `Some(LogStatus)` if valid code, `None` if out of range
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Pending),
            1 => Some(Self::Running),
            2 => Some(Self::Success),
            3 => Some(Self::Failed),
            4 => Some(Self::Timeout),
            5 => Some(Self::Cancelled),
            6 => Some(Self::Evicted),
            7 => Some(Self::TurboFallback),
            _ => None,
        }
    }

    /// Get human-readable status name
    ///
    /// # Returns
    ///
    /// Status as static string reference
    pub fn name(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Running => "Running",
            Self::Success => "Success",
            Self::Failed => "Failed",
            Self::Timeout => "Timeout",
            Self::Cancelled => "Cancelled",
            Self::Evicted => "Evicted",
            Self::TurboFallback => "TurboFallback",
        }
    }
}

impl std::fmt::Display for LogStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminal_status() {
        assert!(LogStatus::Success.is_terminal());
        assert!(LogStatus::Failed.is_terminal());
        assert!(LogStatus::Timeout.is_terminal());
        assert!(LogStatus::Cancelled.is_terminal());
        assert!(LogStatus::Evicted.is_terminal());

        assert!(!LogStatus::Pending.is_terminal());
        assert!(!LogStatus::Running.is_terminal());
    }

    #[test]
    fn test_success_status() {
        assert!(LogStatus::Success.is_success());
        assert!(!LogStatus::Failed.is_success());
        assert!(!LogStatus::Pending.is_success());
    }

    #[test]
    fn test_failure_status() {
        assert!(LogStatus::Failed.is_failure());
        assert!(LogStatus::Timeout.is_failure());
        assert!(LogStatus::Cancelled.is_failure());
        assert!(LogStatus::Evicted.is_failure());

        assert!(!LogStatus::Success.is_failure());
        assert!(!LogStatus::Pending.is_failure());
    }

    #[test]
    fn test_in_progress_status() {
        assert!(LogStatus::Pending.is_in_progress());
        assert!(LogStatus::Running.is_in_progress());

        assert!(!LogStatus::Success.is_in_progress());
        assert!(!LogStatus::Failed.is_in_progress());
    }

    #[test]
    fn test_turbo_related() {
        assert!(LogStatus::TurboFallback.is_turbo_related());
        assert!(!LogStatus::Success.is_turbo_related());
        assert!(!LogStatus::Failed.is_turbo_related());
    }

    #[test]
    fn test_status_codes() {
        assert_eq!(LogStatus::Pending.to_code(), 0);
        assert_eq!(LogStatus::Running.to_code(), 1);
        assert_eq!(LogStatus::Success.to_code(), 2);
        assert_eq!(LogStatus::Failed.to_code(), 3);
        assert_eq!(LogStatus::Timeout.to_code(), 4);
        assert_eq!(LogStatus::Cancelled.to_code(), 5);
        assert_eq!(LogStatus::Evicted.to_code(), 6);
        assert_eq!(LogStatus::TurboFallback.to_code(), 7);
    }

    #[test]
    fn test_status_parsing() {
        assert_eq!(LogStatus::from_code(0), Some(LogStatus::Pending));
        assert_eq!(LogStatus::from_code(2), Some(LogStatus::Success));
        assert_eq!(LogStatus::from_code(7), Some(LogStatus::TurboFallback));
        assert_eq!(LogStatus::from_code(8), None);
        assert_eq!(LogStatus::from_code(255), None);
    }

    #[test]
    fn test_status_names() {
        assert_eq!(LogStatus::Success.name(), "Success");
        assert_eq!(LogStatus::TurboFallback.name(), "TurboFallback");
        assert_eq!(format!("{}", LogStatus::Failed), "Failed");
    }

    #[test]
    fn test_default_status() {
        assert_eq!(LogStatus::default(), LogStatus::Pending);
    }

    #[test]
    fn test_serialization() {
        let status = LogStatus::Success;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"SUCCESS\"");

        let deserialized: LogStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, LogStatus::Success);
    }

    #[test]
    fn test_all_statuses_roundtrip() {
        let all_statuses = [
            LogStatus::Pending,
            LogStatus::Running,
            LogStatus::Success,
            LogStatus::Failed,
            LogStatus::Timeout,
            LogStatus::Cancelled,
            LogStatus::Evicted,
            LogStatus::TurboFallback,
        ];

        for status in &all_statuses {
            let code = status.to_code();
            let parsed = LogStatus::from_code(code).unwrap();
            assert_eq!(*status, parsed);
            assert!(!status.name().is_empty());
        }
    }
}
