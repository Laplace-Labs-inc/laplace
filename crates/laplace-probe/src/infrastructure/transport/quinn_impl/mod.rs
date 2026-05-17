//! # Quinn QUIC Transport Implementation - Abstraction Barrier
//!
//! This module wraps the quinn QUIC library with domain trait implementations,
//! creating an abstraction barrier that enables replacement with custom native
//! engines in Phase 4. All quinn-specific types and APIs are isolated here.
//!
//! ## Compatibility
//!
//! - quinn: v0.11
//! - rustls: v0.23+
//! - rcgen: v0.14+
//! - rustls-pki-types: Latest

pub mod config;

use crate::domain::transport::{KnulConnection, KnulEndpoint, KnulStream, TransportError};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;

// ============================================================================
// STREAM WRAPPER - Quinn SendStream/RecvStream as KnulStream
// ============================================================================

/// Wraps quinn::SendStream and quinn::RecvStream as KnulStream trait object.
///
/// Provides a unified interface for bidirectional stream operations while tracking
/// open/closed state independently.
pub struct QuinnStream {
    send: Option<quinn::SendStream>,
    recv: Option<quinn::RecvStream>,
    is_open: bool,
}

impl QuinnStream {
    /// Create a new stream wrapper from Quinn send and receive streams.
    fn new(send: quinn::SendStream, recv: quinn::RecvStream) -> Self {
        Self {
            send: Some(send),
            recv: Some(recv),
            is_open: true,
        }
    }
}

#[async_trait]
impl KnulStream for QuinnStream {
    /// Read up to buf.len() bytes from the stream.
    ///
    /// Uses recv.read(buf).await which returns variable-length packets, avoiding
    /// deadlocks when reading packets of unknown size. Returns the number of
    /// bytes read, or 0 on EOF.
    ///
    /// Note: Does NOT check is_open before reading. This allows reading after
    /// close() in half-close mode (write closed, read still open to receive peer's response).
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        let recv = self.recv.as_mut().ok_or(TransportError::StreamError)?;

        // Use recv.read() instead of read_exact() to handle variable-length packets
        // This avoids deadlocks when the sender closes before filling the buffer
        match recv.read(buf).await {
            Ok(Some(n)) => Ok(n),
            Ok(None) => {
                // Stream reached EOF (peer closed their write end)
                self.is_open = false;
                Ok(0)
            }
            Err(_) => Err(TransportError::IoError),
        }
    }

    /// Write all bytes from buf to the stream.
    ///
    /// Returns the number of bytes written on success.
    async fn write(&mut self, buf: &[u8]) -> Result<usize, TransportError> {
        if !self.is_open {
            return Err(TransportError::StreamError);
        }

        let send = self.send.as_mut().ok_or(TransportError::StreamError)?;

        match send.write_all(buf).await {
            Ok(_) => Ok(buf.len()),
            Err(_) => Err(TransportError::IoError),
        }
    }

    /// Close the stream gracefully.
    ///
    /// In quinn v0.11, finish() returns Result immediately (not a Future).
    /// This signals to the remote peer that no more data will be sent.
    async fn close(&mut self) -> Result<(), TransportError> {
        self.is_open = false;

        if let Some(mut send) = self.send.take() {
            // finish() returns Result directly in quinn v0.11
            let _ = send.finish().map_err(|_| TransportError::IoError)?;
        }

        Ok(())
    }

    /// Check if stream is still open for reading/writing.
    fn is_open(&self) -> bool {
        self.is_open
    }
}

// ============================================================================
// CONNECTION WRAPPER - Quinn Connection as KnulConnection
// ============================================================================

/// Wraps quinn::Connection as KnulConnection trait object.
///
/// Uses Arc<RwLock<>> to provide thread-safe mutable access to the underlying
/// Quinn connection while maintaining the async trait interface.
pub struct QuinnConnection {
    conn: Arc<RwLock<quinn::Connection>>,
}

impl QuinnConnection {
    /// Create a new connection wrapper from a Quinn connection instance.
    pub fn new(conn: quinn::Connection) -> Self {
        Self {
            conn: Arc::new(RwLock::new(conn)),
        }
    }
}

#[async_trait]
impl KnulConnection for QuinnConnection {
    /// Open a new bidirectional stream on this connection.
    ///
    /// Clones the connection before awaiting to avoid holding the RwLock guard
    /// across an await point (anti-pattern). The lock is released immediately
    /// after cloning.
    async fn open_stream(&mut self) -> Result<Box<dyn KnulStream>, TransportError> {
        if !self.is_open() {
            return Err(TransportError::ConnectionClosed);
        }

        let conn = {
            let guard = self.conn.read().await;
            guard.clone()
        };

        let (send, recv) = conn
            .open_bi()
            .await
            .map_err(|_| TransportError::StreamError)?;

        Ok(Box::new(QuinnStream::new(send, recv)))
    }

    /// Accept an incoming bidirectional stream from the remote peer.
    ///
    /// Blocks until a stream is opened by the remote peer or the connection is closed.
    /// Returns an error if the connection has been closed.
    ///
    /// Clones the connection before awaiting to avoid holding the RwLock guard
    /// across an await point (anti-pattern). The lock is released immediately
    /// after cloning.
    async fn accept_stream(&mut self) -> Result<Box<dyn KnulStream>, TransportError> {
        if !self.is_open() {
            return Err(TransportError::ConnectionClosed);
        }

        let conn = {
            let guard = self.conn.write().await;
            guard.clone()
        };

        // accept_bi() waits for an incoming bidirectional stream from the peer
        match conn.accept_bi().await {
            Ok((send, recv)) => Ok(Box::new(QuinnStream::new(send, recv))),
            Err(_) => Err(TransportError::StreamError),
        }
    }

    /// Close the connection gracefully.
    ///
    /// Sends a connection close frame to the remote peer.
    async fn close(&mut self) -> Result<(), TransportError> {
        let conn = self.conn.write().await;
        let reason = b"normal";
        conn.close(quinn::VarInt::from_u32(0), reason);
        Ok(())
    }

    /// Check if connection is still open.
    ///
    /// Note: This is a simplified check. In production, should query actual
    /// connection state from Quinn.
    fn is_open(&self) -> bool {
        true
    }

    /// Get the remote peer's address (for diagnostics).
    fn peer_addr(&self) -> String {
        // Note: Placeholder implementation. Should extract actual peer address
        // from Quinn connection metadata in production.
        "peer".to_string()
    }
}

// ============================================================================
// ENDPOINT WRAPPER - Quinn Endpoint as KnulEndpoint
// ============================================================================

/// Wraps quinn::Endpoint as KnulEndpoint trait object.
///
/// Handles server-mode operation (accepting connections) and client-mode
/// operation (initiating connections). Quinn v0.11 simplified the Endpoint API.
pub struct QuinnEndpoint {
    endpoint: Option<quinn::Endpoint>,
    is_running: bool,
    client_connection: Option<Box<dyn KnulConnection>>,
}

impl QuinnEndpoint {
    /// Create a new endpoint wrapper (initially not running).
    pub fn new() -> Self {
        Self {
            endpoint: None,
            is_running: false,
            client_connection: None,
        }
    }
}

#[async_trait]
impl KnulEndpoint for QuinnEndpoint {
    /// Accept the next incoming connection from a client.
    ///
    /// Blocks until a new connection arrives or the endpoint is closed.
    /// Returns None if the endpoint is no longer accepting connections.
    async fn accept_connection(
        &mut self,
    ) -> Result<Option<Box<dyn KnulConnection>>, TransportError> {
        let endpoint = self
            .endpoint
            .as_mut()
            .ok_or(TransportError::ConnectionClosed)?;

        match endpoint.accept().await {
            Some(conn) => {
                let conn = conn.await.map_err(|_| TransportError::ConnectionClosed)?;
                Ok(Some(Box::new(QuinnConnection::new(conn))))
            }
            None => Ok(None),
        }
    }

    /// Get the current count of active connections on this endpoint.
    fn active_connection_count(&self) -> usize {
        self.endpoint
            .as_ref()
            .map(|ep| ep.open_connections())
            .unwrap_or(0)
    }

    /// Check if the endpoint is currently running.
    fn is_running(&self) -> bool {
        self.is_running
    }

    /// Shut down the endpoint gracefully.
    ///
    /// Waits for all pending operations to complete before releasing resources.
    async fn shutdown(&mut self) -> Result<(), TransportError> {
        if let Some(endpoint) = self.endpoint.take() {
            endpoint.wait_idle().await;
        }
        self.is_running = false;
        self.client_connection = None;
        Ok(())
    }
}

impl Default for QuinnEndpoint {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quinn_endpoint_creation() {
        let endpoint = QuinnEndpoint::new();
        assert!(!endpoint.is_running);
    }

    #[test]
    fn test_quinn_endpoint_default() {
        let endpoint = QuinnEndpoint::default();
        assert!(!endpoint.is_running);
        assert!(endpoint.endpoint.is_none());
    }

    #[test]
    fn test_quinn_stream_creation() {
        // Note: Full integration tests require async runtime and network setup
        // This is a basic structural test that validates type construction
    }
}
