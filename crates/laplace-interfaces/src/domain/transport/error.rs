//! # Transport Layer Errors
//!
//! Defines the complete taxonomy of transport-level errors that can occur
//! during QUIC operations, stream management, and connection lifecycle events.
//! Each variant maps to a specific Laplace error code for unified error handling
//! across the system boundary.

use std::fmt;

/// Connection-level errors returned by transport operations.
///
/// Represents the complete taxonomy of transport layer errors. Each variant
/// indicates a specific failure condition that the caller must handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportError {
    /// Connection is closed or unreachable.
    ///
    /// The connection has been gracefully closed or is no longer available.
    /// This error is retryable; the client should establish a new connection.
    ConnectionClosed,

    /// Stream operation failed.
    ///
    /// A stream read, write, or state transition failed. This may indicate
    /// stream corruption, peer reset, or protocol violation.
    /// Recommendation: Close stream and open new one.
    StreamError,

    /// Configuration is invalid.
    ///
    /// The provided configuration does not meet constraints (e.g., invalid addresses,
    /// unsupported TLS versions, out-of-range parameters).
    /// Recommendation: Fix configuration; do not retry.
    InvalidConfig,

    /// Internal I/O error.
    ///
    /// Underlying I/O system returned an error (e.g., socket error, file descriptor
    /// exhaustion). May be transient.
    /// Recommendation: Retry with backoff.
    IoError,

    /// TLS handshake failed.
    ///
    /// The TLS handshake with the peer failed due to certificate validation,
    /// cipher mismatch, or protocol violation.
    /// Recommendation: Verify TLS configuration; may be retryable if temporary.
    TlsError,

    /// Operation timed out.
    ///
    /// A transport operation exceeded its time limit (e.g., connection timeout,
    /// read timeout).
    /// Recommendation: Retry with longer deadline.
    Timeout,
}

impl TransportError {
    /// Convert transport error to a Laplace error code.
    ///
    /// Maps transport-specific errors to the unified Laplace error code space
    /// for consistent error handling across the system.
    ///
    /// # Returns
    ///
    /// A u32 error code suitable for FFI boundary and Protobuf serialization.
    pub fn to_error_code(&self) -> u32 {
        match self {
            TransportError::ConnectionClosed => 5001, // ConnectionFailed
            TransportError::StreamError => 5000,      // NetworkError
            TransportError::InvalidConfig => 4001,    // InvalidRequest
            TransportError::IoError => 5000,          // NetworkError
            TransportError::TlsError => 4000,         // HandshakeFailed
            TransportError::Timeout => 2000,          // Timeout
        }
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportError::ConnectionClosed => write!(f, "Connection closed"),
            TransportError::StreamError => write!(f, "Stream error"),
            TransportError::InvalidConfig => write!(f, "Invalid configuration"),
            TransportError::IoError => write!(f, "I/O error"),
            TransportError::TlsError => write!(f, "TLS error"),
            TransportError::Timeout => write!(f, "Operation timeout"),
        }
    }
}

impl std::error::Error for TransportError {}
