//! # Domain Model: Tenant Error Types
//!
//! Business rule violations and operational failures at the domain level.
//! Infrastructure errors (V8 crashes, network failures) belong in adapter layers.
//!
//! Tenant errors map deterministically to FFI boundary codes via `to_proto_code()`,
//! enabling consistent error handling across language boundaries and tenancy boundaries.

use crate::domain::tenant::TenantTier;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tenant Error Types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Tenant-related domain errors.
///
/// These represent business rule violations and operational failures at the domain level.
/// Each variant maps deterministically to an FFI boundary error code for consistent
/// error handling across the Rust-TypeScript boundary.
///
/// # Spec Compliance
///
/// - Spec-008: Error codes for SDK propagation
/// - Sovereign-002: Tenant isolation enforcement
/// - Sovereign-003: Quota and limit violations
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Error",
        link = "LEP-0017-laplace-interfaces-error_taxonomy"
    )
)]
#[derive(Debug, Clone, thiserror::Error)]
pub enum TenantError {
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // Client Errors (1000-1099)
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    /// Tenant does not exist or has been deleted.
    #[error("Tenant not found: {0}")]
    NotFound(String),

    /// Tenant is inactive (suspended, banned, or unpaid).
    #[error("Tenant inactive: {0}")]
    Inactive(String),

    /// Invalid tenant ID format.
    #[error("Invalid tenant ID: {0}")]
    InvalidId(String),

    /// Invalid tier change attempt.
    #[error("Invalid tier change from {current} to {requested}")]
    InvalidTierChange {
        /// Current tenant tier.
        current: TenantTier,
        /// Requested tenant tier.
        requested: TenantTier,
    },

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // Quota/Limit Errors (2000-2099)
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    /// Quota exceeded (C-002 enforcement).
    #[error("Quota exceeded for tenant: {0}")]
    QuotaExceeded(String),

    /// Semaphore acquisition timeout (C-002 enforcement).
    #[error("Acquire timeout for tenant: {0}")]
    AcquireTimeout(String),

    /// Execution timeout (C-003 enforcement).
    #[error(
        "Execution timeout for tenant {tenant_id}: {elapsed_ms}ms exceeded limit of {limit_ms}ms"
    )]
    ExecutionTimeout {
        /// Tenant identifier.
        tenant_id: String,
        /// Time limit in milliseconds.
        limit_ms: u64,
        /// Elapsed time in milliseconds.
        elapsed_ms: u64,
    },

    /// V8 runtime error (adapter layer propagated).
    #[error("Runtime error: {0}")]
    RuntimeError(String),

    /// Path access denied (Spec-002 enforcement).
    #[error("Path access denied: {0}")]
    PathDenied(String),

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // Turbo-Specific Errors (3000-3099)
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    /// Shared memory pool exhausted.
    ///
    /// This occurs when all Turbo slots are occupied and no fallback is available.
    #[error("Turbo pool exhausted for tenant {tenant_id}: {available} slots available, {required} required")]
    TurboPoolExhausted {
        /// Tenant identifier.
        tenant_id: String,
        /// Number of available slots.
        available: usize,
        /// Number of required slots.
        required: usize,
    },

    /// Shared memory slot in invalid state.
    ///
    /// This indicates a state machine violation in the slot lifecycle.
    #[error("Turbo slot state error for tenant {tenant_id}: expected {expected}, found {actual}")]
    TurboSlotStateError {
        /// Tenant identifier.
        tenant_id: String,
        /// Expected state.
        expected: String,
        /// Actual state.
        actual: String,
    },

    /// Shared memory corruption detected.
    ///
    /// This is a critical error indicating memory safety violation.
    #[error("Turbo slot corruption detected for tenant {tenant_id}: magic mismatch")]
    TurboSlotCorruption {
        /// Tenant identifier.
        tenant_id: String,
    },

    /// Turbo acceleration not available for tier.
    ///
    /// User tried to access Turbo features without qualifying tier.
    #[error("Turbo acceleration not available for tenant {tenant_id} (tier: {tier})")]
    TurboNotAvailable {
        /// Tenant identifier.
        tenant_id: String,
        /// Current tenant tier.
        tier: TenantTier,
    },

    /// Turbo slot allocation failed.
    ///
    /// This indicates a low-level memory allocation failure.
    #[error("Turbo slot allocation failed for tenant {tenant_id}: {reason}")]
    TurboAllocationFailed {
        /// Tenant identifier.
        tenant_id: String,
        /// Failure reason.
        reason: String,
    },

    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    // Internal Errors (9000-9999)
    // ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
    /// Generic internal error (should be rare).
    #[error("Internal error: {0}")]
    Internal(String),
}

impl TenantError {
    /// Convert to FFI boundary error code (Spec-008 compliance).
    ///
    /// Maps each `TenantError` variant to the corresponding `LaplaceError` code
    /// in the FFI boundary enumeration. This deterministic mapping enables
    /// consistent error handling across language boundaries.
    ///
    /// # Error Code Mapping
    ///
    /// - 1000-1099: Client errors (user-facing)
    /// - 2000-2099: Quota/limit errors
    /// - 3000-3099: Turbo-specific errors
    /// - 9000-9999: Internal errors
    ///
    /// # Returns
    ///
    /// Numeric error code for SDK propagation.
    pub fn to_proto_code(&self) -> i32 {
        match self {
            Self::NotFound(_) => 1001,
            Self::Inactive(_) => 1002,
            Self::InvalidId(_) => 1003,
            Self::InvalidTierChange { .. } => 1004,

            Self::QuotaExceeded(_) => 2001,
            Self::AcquireTimeout(_) => 2002,
            Self::ExecutionTimeout { .. } => 2003,
            Self::RuntimeError(_) => 2004,
            Self::PathDenied(_) => 2005,

            Self::TurboPoolExhausted { .. } => 3001,
            Self::TurboSlotStateError { .. } => 3002,
            Self::TurboSlotCorruption { .. } => 3003,
            Self::TurboNotAvailable { .. } => 3004,
            Self::TurboAllocationFailed { .. } => 3005,

            Self::Internal(_) => 9000,
        }
    }

    /// Convert to error category string.
    ///
    /// Returns a semantic category identifier for structured error responses.
    /// Used by adapters to classify errors for logging, metrics, and client-side handling.
    ///
    /// # Returns
    ///
    /// Error category string (one of CLIENT_ERROR, QUOTA_EXCEEDED, EXECUTION_ERROR, TURBO_ERROR, INTERNAL_ERROR).
    pub fn to_proto_category(&self) -> &'static str {
        match self {
            Self::NotFound(_)
            | Self::Inactive(_)
            | Self::InvalidId(_)
            | Self::InvalidTierChange { .. } => "CLIENT_ERROR",

            Self::QuotaExceeded(_) | Self::AcquireTimeout(_) | Self::ExecutionTimeout { .. } => {
                "QUOTA_EXCEEDED"
            }

            Self::RuntimeError(_) | Self::PathDenied(_) => "EXECUTION_ERROR",

            Self::TurboPoolExhausted { .. }
            | Self::TurboSlotStateError { .. }
            | Self::TurboSlotCorruption { .. }
            | Self::TurboNotAvailable { .. }
            | Self::TurboAllocationFailed { .. } => "TURBO_ERROR",

            Self::Internal(_) => "INTERNAL_ERROR",
        }
    }

    /// Check if error is recoverable (retryable).
    ///
    /// Recoverable errors represent transient conditions that may succeed
    /// if the operation is attempted again, typically with exponential backoff.
    ///
    /// # Returns
    ///
    /// `true` if error is transient and retryable, `false` if permanent.
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            Self::QuotaExceeded(_)
                | Self::AcquireTimeout(_)
                | Self::TurboPoolExhausted { .. }
                | Self::TurboAllocationFailed { .. }
        )
    }

    /// Check if error indicates tier upgrade would help.
    ///
    /// Identifies errors that would be resolved by upgrading to a higher tenant tier.
    /// Used for marketing prompts and upsell opportunities in SDK error handling.
    ///
    /// # Returns
    ///
    /// `true` if upgrading tier would resolve the error.
    pub fn suggests_upgrade(&self) -> bool {
        matches!(
            self,
            Self::QuotaExceeded(_) | Self::TurboNotAvailable { .. } | Self::ExecutionTimeout { .. }
        )
    }

    /// Check if error is Turbo acceleration-specific.
    ///
    /// Identifies errors related to the zero-copy shared memory acceleration feature.
    /// Used for diagnostic logging and feature-specific error handling.
    ///
    /// # Returns
    ///
    /// `true` if error is specific to Turbo acceleration.
    pub fn is_turbo_error(&self) -> bool {
        matches!(
            self,
            Self::TurboPoolExhausted { .. }
                | Self::TurboSlotStateError { .. }
                | Self::TurboSlotCorruption { .. }
                | Self::TurboNotAvailable { .. }
                | Self::TurboAllocationFailed { .. }
        )
    }

    /// Extract tenant ID from error (if available).
    ///
    /// Retrieves the tenant identifier embedded in the error for tracing and correlation.
    /// Returns "unknown" if the error variant does not include a tenant ID.
    ///
    /// # Returns
    ///
    /// Tenant ID string, or "unknown" if not available.
    #[allow(dead_code)]
    fn extract_tenant_id(&self) -> String {
        match self {
            Self::NotFound(id)
            | Self::Inactive(id)
            | Self::InvalidId(id)
            | Self::QuotaExceeded(id)
            | Self::AcquireTimeout(id) => id.clone(),

            Self::ExecutionTimeout { tenant_id, .. } => tenant_id.clone(),

            Self::TurboPoolExhausted { tenant_id, .. }
            | Self::TurboSlotStateError { tenant_id, .. }
            | Self::TurboSlotCorruption { tenant_id }
            | Self::TurboNotAvailable { tenant_id, .. }
            | Self::TurboAllocationFailed { tenant_id, .. } => tenant_id.clone(),

            _ => "unknown".to_string(),
        }
    }

    // @public-todo: Restore into_proto when Protobuf compilation is hooked up.
    // This method was previously implemented to convert TenantError to a Protobuf LaplaceError message.
    // Once the Protobuf Rust code is generated and available in this crate, this method should be
    // restored to enable serialization of tenant errors across the FFI boundary.
    //
    // pub fn into_proto(self, request_id: String, trace_id: String) -> LaplaceError {
    //     let tenant_id = self.extract_tenant_id();
    //     let code = self.to_proto_code();
    //     let category = self.to_proto_category();
    //     let message = self.to_string();
    //     // ... Protobuf serialization logic
    // }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes() {
        assert_eq!(TenantError::NotFound("x".into()).to_proto_code(), 1001);
        assert_eq!(TenantError::Inactive("x".into()).to_proto_code(), 1002);
        assert_eq!(TenantError::InvalidId("x".into()).to_proto_code(), 1003);

        assert_eq!(TenantError::QuotaExceeded("x".into()).to_proto_code(), 2001);
        assert_eq!(
            TenantError::AcquireTimeout("x".into()).to_proto_code(),
            2002
        );
        assert_eq!(
            TenantError::ExecutionTimeout {
                tenant_id: "x".into(),
                limit_ms: 100,
                elapsed_ms: 150
            }
            .to_proto_code(),
            2003
        );

        assert_eq!(
            TenantError::TurboPoolExhausted {
                tenant_id: "x".into(),
                available: 0,
                required: 1
            }
            .to_proto_code(),
            3001
        );
        assert_eq!(
            TenantError::TurboSlotStateError {
                tenant_id: "x".into(),
                expected: "idle".into(),
                actual: "active".into()
            }
            .to_proto_code(),
            3002
        );
        assert_eq!(
            TenantError::TurboSlotCorruption {
                tenant_id: "x".into()
            }
            .to_proto_code(),
            3003
        );

        assert_eq!(TenantError::Internal("x".into()).to_proto_code(), 9000);
    }

    #[test]
    fn error_categories() {
        assert_eq!(
            TenantError::NotFound("x".into()).to_proto_category(),
            "CLIENT_ERROR"
        );
        assert_eq!(
            TenantError::QuotaExceeded("x".into()).to_proto_category(),
            "QUOTA_EXCEEDED"
        );
        assert_eq!(
            TenantError::RuntimeError("x".into()).to_proto_category(),
            "EXECUTION_ERROR"
        );
        assert_eq!(
            TenantError::TurboPoolExhausted {
                tenant_id: "x".into(),
                available: 0,
                required: 1
            }
            .to_proto_category(),
            "TURBO_ERROR"
        );
        assert_eq!(
            TenantError::Internal("x".into()).to_proto_category(),
            "INTERNAL_ERROR"
        );
    }

    #[test]
    fn error_recoverability() {
        assert!(TenantError::QuotaExceeded("x".into()).is_recoverable());
        assert!(TenantError::AcquireTimeout("x".into()).is_recoverable());
        assert!(TenantError::TurboPoolExhausted {
            tenant_id: "x".into(),
            available: 0,
            required: 1
        }
        .is_recoverable());

        assert!(!TenantError::NotFound("x".into()).is_recoverable());
        assert!(!TenantError::TurboSlotCorruption {
            tenant_id: "x".into()
        }
        .is_recoverable());
    }

    #[test]
    fn upgrade_suggestions() {
        assert!(TenantError::QuotaExceeded("x".into()).suggests_upgrade());
        assert!(TenantError::TurboNotAvailable {
            tenant_id: "x".into(),
            tier: TenantTier::Free
        }
        .suggests_upgrade());
        assert!(TenantError::ExecutionTimeout {
            tenant_id: "x".into(),
            limit_ms: 100,
            elapsed_ms: 150
        }
        .suggests_upgrade());

        assert!(!TenantError::NotFound("x".into()).suggests_upgrade());
        assert!(!TenantError::Internal("x".into()).suggests_upgrade());
    }

    #[test]
    fn turbo_error_detection() {
        assert!(TenantError::TurboPoolExhausted {
            tenant_id: "x".into(),
            available: 0,
            required: 1
        }
        .is_turbo_error());

        assert!(TenantError::TurboSlotStateError {
            tenant_id: "x".into(),
            expected: "idle".into(),
            actual: "active".into()
        }
        .is_turbo_error());

        assert!(TenantError::TurboNotAvailable {
            tenant_id: "x".into(),
            tier: TenantTier::Standard
        }
        .is_turbo_error());

        assert!(!TenantError::QuotaExceeded("x".into()).is_turbo_error());
        assert!(!TenantError::RuntimeError("x".into()).is_turbo_error());
    }

    #[test]
    fn tenant_id_extraction() {
        let err1 = TenantError::NotFound("tenant-123".into());
        assert_eq!(err1.extract_tenant_id(), "tenant-123");

        let err2 = TenantError::TurboPoolExhausted {
            tenant_id: "tenant-456".into(),
            available: 0,
            required: 1,
        };
        assert_eq!(err2.extract_tenant_id(), "tenant-456");

        let err3 = TenantError::Internal("some error".into());
        assert_eq!(err3.extract_tenant_id(), "unknown");
    }

    #[test]
    fn error_display() {
        let err = TenantError::TurboPoolExhausted {
            tenant_id: "test".into(),
            available: 5,
            required: 10,
        };
        let msg = format!("{}", err);
        assert!(msg.contains("Turbo pool exhausted"));
        assert!(msg.contains("test"));
        assert!(msg.contains("5"));
        assert!(msg.contains("10"));
    }
}
