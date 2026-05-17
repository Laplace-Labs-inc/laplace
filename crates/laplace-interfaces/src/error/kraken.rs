//! Kraken error types (Axiom verification layer)
//!
//! `KrakenError` covers all error variants from Kraken DNA operations:
//! seed distribution, VU state machine, scenario execution, and network transport.

#![cfg(feature = "twin")]

use crate::domain::entropy::types::{ContextId, LocalSeed};
use std::fmt;

/// Error types for Kraken DNA and VU operations
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KrakenError {
    /// Maximum VU quota exceeded
    QuotaExceeded {
        /// Current number of registered VUs at the time of the failure.
        current: usize,
        /// Maximum VUs permitted by the scenario configuration.
        max: usize,
    },

    /// VU ID already registered
    DuplicateRegistration(ContextId),

    /// VU not found in registry
    NotFound(ContextId),

    /// Invalid seed assignment (verification failed)
    InvalidSeedAssignment {
        /// VU whose deterministic seed failed verification.
        vu_id: ContextId,
        /// Seed derived from `DeriveLocalSeed(vu_id)` — the correct value.
        expected: LocalSeed,
        /// Seed actually stored in the VU state — the erroneous value.
        got: LocalSeed,
    },

    /// RNG state corrupted
    RngStateCorrupted(String),

    /// VirtualClock error
    ClockError(String),

    /// Invalid state transition attempted in VU state machine
    InvalidTransition {
        /// Name of the VU state in which the invalid transition was attempted.
        from_state: String,
        /// Name of the action that triggered the invalid transition.
        action: String,
        /// Explanation of why the transition is disallowed.
        reason: String,
    },

    /// Network-level errors from the virtual transport layer
    NetworkError {
        /// Description of the network-level failure.
        reason: String,
    },

    /// Scenario execution error
    InvalidScenario(String),

    /// Generic operational error
    Other(String),
}

impl fmt::Display for KrakenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QuotaExceeded { current, max } => {
                write!(f, "VU quota exceeded: {}/{}", current, max)
            }
            Self::DuplicateRegistration(vu) => {
                write!(f, "VU {} already registered", vu)
            }
            Self::NotFound(vu) => {
                write!(f, "VU {} not found", vu)
            }
            Self::InvalidSeedAssignment {
                vu_id,
                expected,
                got,
            } => {
                write!(
                    f,
                    "Invalid seed for {}: expected {}, got {}",
                    vu_id, expected, got
                )
            }
            Self::RngStateCorrupted(msg) => {
                write!(f, "RNG state corrupted: {}", msg)
            }
            Self::ClockError(msg) => {
                write!(f, "Clock error: {}", msg)
            }
            Self::InvalidTransition {
                from_state,
                action,
                reason,
            } => {
                write!(
                    f,
                    "Invalid transition from state '{}' with action '{}': {}",
                    from_state, action, reason
                )
            }
            Self::NetworkError { reason } => {
                write!(f, "Network error: {}", reason)
            }
            Self::InvalidScenario(msg) => {
                write!(f, "Invalid scenario: {}", msg)
            }
            Self::Other(msg) => {
                write!(f, "Kraken error: {}", msg)
            }
        }
    }
}

impl std::error::Error for KrakenError {}

/// Result type for Kraken DNA and VU operations
pub type Result<T> = std::result::Result<T, KrakenError>;
