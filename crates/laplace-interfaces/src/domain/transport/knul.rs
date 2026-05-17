//! # QUIC Transport Layer (KNUL) Contracts
//!
//! Defines the authoritative trait contracts for the Kernel Networking Utility Link (KNUL),
//! Laplace's QUIC-based transport layer. These traits establish the interfaces that all
//! network implementations must satisfy, enabling deterministic behavior and zero-copy
//! data transfer for high-performance scenarios.
//!
//! This module is part of the domain contract layer and contains only trait definitions.
//! Concrete implementations belong in `laplace-axiom` and other implementation crates.

use crate::error::TransportError;
use async_trait::async_trait;

/// Represents a single bidirectional QUIC stream for data transfer.
///
/// A stream is a lightweight, multiplexed channel within a QUIC connection.
/// Multiple streams can operate concurrently on a single connection without
/// blocking each other. Streams are ordered—data is delivered in order,
/// and closing the stream signals end-of-file to the peer.
///
/// # Contract
///
/// Both peers have independent close states (send-close and receive-close).
/// Reading from a closed stream returns 0 (EOF). Writing to a closed stream
/// returns an error. A stream may be reset by either peer, resulting in `StreamError`.
#[async_trait]
pub trait KnulStream: Send + Sync {
    /// Read up to `buf.len()` bytes from the stream into `buf`.
    ///
    /// Reads data that has been received from the peer. Blocks until data
    /// becomes available or the stream is closed. Returns the number of bytes
    /// actually read, or 0 if the stream has reached EOF (peer closed their end).
    ///
    /// # Arguments
    ///
    /// * `buf` - Mutable buffer to fill with data
    ///
    /// # Returns
    ///
    /// - `Ok(0)`: End of stream (peer closed their write end)
    /// - `Ok(n)` where n > 0: Successfully read n bytes
    /// - `Err`: Stream error or other I/O failure
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError>;

    /// Write all bytes from `buf` to the stream.
    ///
    /// Sends data to the peer. The write is buffered; this call returns
    /// after the data has been queued, not necessarily delivered.
    ///
    /// # Arguments
    ///
    /// * `buf` - Data to send
    ///
    /// # Returns
    ///
    /// - `Ok(n)`: Successfully queued n bytes (typically n == buf.len())
    /// - `Err`: Connection closed, stream reset, or other I/O failure
    async fn write(&mut self, buf: &[u8]) -> Result<usize, TransportError>;

    /// Close the stream gracefully.
    ///
    /// Signals end-of-stream to the peer and disallows further writes.
    /// Further reads may still return data sent by the peer before their close.
    /// After close completes, subsequent `read` will return 0 and `write` will error.
    ///
    /// # Returns
    ///
    /// - `Ok(())`: Close initiated successfully
    /// - `Err`: Already closed, I/O error, or stream reset
    async fn close(&mut self) -> Result<(), TransportError>;

    /// Check if stream is still open.
    ///
    /// A stream is considered open if it has not been closed by the local side
    /// and has not been reset. Note that the peer may have closed their side
    /// independently.
    ///
    /// # Returns
    ///
    /// `true` if stream can be written to, `false` if closed or reset.
    fn is_open(&self) -> bool;
}

/// Represents a QUIC connection with one or more streams.
///
/// A connection is a long-lived endpoint-to-endpoint communication channel.
/// It encapsulates the TLS session, handles packet retransmission and congestion
/// control, and multiplexes multiple streams. A connection may host up to 2^63
/// concurrent streams (one direction) and up to 2^63 bytes of data per stream.
///
/// # Contract
///
/// Opening streams does not block other operations (non-blocking API).
/// Streams are independent; closing one stream does not affect others.
/// Closing the connection gracefully closes all streams.
/// The connection maintains a single TLS session shared by all streams.
#[async_trait]
pub trait KnulConnection: Send + Sync {
    /// Open a new bidirectional stream on this connection.
    ///
    /// Creates and returns a new stream. The connection must be open;
    /// returns an error if the connection is already closed.
    ///
    /// # Returns
    ///
    /// - `Ok(stream)`: Successfully opened a new stream
    /// - `Err`: Connection closed, too many streams, or configuration error
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let mut stream = connection.open_stream().await?;
    /// stream.write(b"Hello").await?;
    /// ```
    async fn open_stream(&mut self) -> Result<Box<dyn KnulStream>, TransportError>;

    /// Accept an incoming bidirectional stream from the remote peer.
    ///
    /// Blocks until a stream is opened by the remote peer or the connection is closed.
    /// Returns `None` (as `Ok(None)`) if the connection has been closed by either side.
    /// This method is primarily used in server mode to receive client-initiated streams.
    ///
    /// # Returns
    ///
    /// - `Ok(stream)`: Successfully accepted an incoming stream from peer
    /// - `Err`: Connection closed, stream configuration error, or I/O failure
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let mut stream = connection.accept_stream().await?;
    /// let mut buf = vec![0u8; 1024];
    /// let n = stream.read(&mut buf).await?;
    /// ```
    async fn accept_stream(&mut self) -> Result<Box<dyn KnulStream>, TransportError>;

    /// Close the entire connection gracefully.
    ///
    /// Sends a close frame to the peer, which will close all active streams.
    /// Outstanding streams will receive errors or EOF on subsequent operations.
    /// This is a cooperative shutdown; the peer acknowledges closure.
    ///
    /// # Returns
    ///
    /// - `Ok(())`: Close initiated successfully
    /// - `Err`: Already closed or I/O error
    async fn close(&mut self) -> Result<(), TransportError>;

    /// Check if connection is still open.
    ///
    /// A connection is considered open if the local side has not initiated
    /// closure and no fatal error has occurred.
    ///
    /// # Returns
    ///
    /// `true` if new operations can be initiated, `false` otherwise.
    fn is_open(&self) -> bool;

    /// Get connection's remote peer address as a string.
    ///
    /// Returns the peer's address in human-readable format (e.g., "192.0.2.1:4433").
    /// Used for diagnostics, logging, and address-based authorization.
    ///
    /// # Returns
    ///
    /// Human-readable peer address string.
    fn peer_addr(&self) -> String;
}

/// Represents a QUIC endpoint (server or client mode).
///
/// An endpoint is a Laplace transport instance that can operate as either a
/// server (accepting incoming connections) or a client (initiating connections).
/// A single endpoint can manage multiple concurrent connections.
///
/// # Modes
///
/// Server mode binds to a local address, listens for incoming connections,
/// and accepts them. Client mode initiates outbound connections to remote servers.
///
/// # Contract
///
/// An endpoint in server mode cannot initiate connections.
/// An endpoint in client mode cannot accept incoming connections.
/// Multiple connections are managed independently; closing one does not affect others.
/// The endpoint maintains global state (routing, connection pools, TLS sessions).
#[async_trait]
pub trait KnulEndpoint: Send + Sync {
    /// Accept next incoming connection from a client.
    ///
    /// Blocks until a connection arrives or the endpoint is closed. Returns `None`
    /// if the endpoint has shut down. This call is used in server mode to accept
    /// incoming client connections, and in client mode after successful `connect_client`.
    ///
    /// # Returns
    ///
    /// - `Ok(Some(conn))`: Successfully accepted a connection
    /// - `Ok(None)`: Endpoint is shutting down; no more connections will arrive
    /// - `Err`: Internal error or fatal condition
    async fn accept_connection(
        &mut self,
    ) -> Result<Option<Box<dyn KnulConnection>>, TransportError>;

    /// Get count of currently active connections.
    ///
    /// Returns a snapshot of the number of connections currently managed by this endpoint.
    /// Useful for monitoring, diagnostics, and connection pool management.
    ///
    /// # Returns
    ///
    /// Number of active connections (includes both client and server connections).
    fn active_connection_count(&self) -> usize;

    /// Check if endpoint is running.
    ///
    /// Returns `true` if the endpoint is actively listening or can accept new connections.
    /// Returns `false` if the endpoint has been shut down.
    ///
    /// # Returns
    ///
    /// `true` if running and accepting operations, `false` if shut down.
    fn is_running(&self) -> bool;

    /// Shutdown the endpoint gracefully.
    ///
    /// Closes the listening socket (in server mode) and initiates graceful closure
    /// of all active connections. Existing streams will receive errors or EOF on
    /// further operations. This is a cooperative shutdown; the endpoint will not
    /// forcefully terminate active streams.
    ///
    /// # Returns
    ///
    /// - `Ok(())`: Shutdown initiated successfully
    /// - `Err`: Already shut down or error during shutdown
    async fn shutdown(&mut self) -> Result<(), TransportError>;
}
