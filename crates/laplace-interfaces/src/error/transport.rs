//! # Transport Layer Errors
//!
//! Connection-level errors returned by KNUL transport operations.
//! Defines the error taxonomy for QUIC stream, connection, and endpoint operations.

use std::fmt;

/// Connection-level errors returned by transport operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportError {
    /// Connection is closed or unreachable
    ConnectionClosed,
    /// Stream operation failed
    StreamError,
    /// Configuration invalid
    InvalidConfig,
    /// Internal I/O error
    IoError,
    /// TLS handshake failed
    TlsError,
    /// Operation timed out
    Timeout,
}

impl TransportError {
    /// Convert to laplace_core error code
    pub fn to_error_code(&self) -> u32 {
        match self {
            TransportError::ConnectionClosed => 2001,
            TransportError::StreamError => 2002,
            TransportError::InvalidConfig => 2003,
            TransportError::IoError => 2004,
            TransportError::TlsError => 2005,
            TransportError::Timeout => 2006,
        }
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportError::ConnectionClosed => write!(f, "Connection closed or unreachable"),
            TransportError::StreamError => write!(f, "Stream operation failed"),
            TransportError::InvalidConfig => write!(f, "Configuration invalid"),
            TransportError::IoError => write!(f, "Internal I/O error"),
            TransportError::TlsError => write!(f, "TLS handshake failed"),
            TransportError::Timeout => write!(f, "Operation timed out"),
        }
    }
}

impl std::error::Error for TransportError {}
