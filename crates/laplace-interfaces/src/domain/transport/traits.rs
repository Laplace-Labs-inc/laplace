//! # High-Level Transport Abstractions
//!
//! Defines the primary trait contracts that abstract network transport implementations.
//! The SovereignTransport trait enables kernel-independent transport backends
//! (QUIC, HTTP/3, etc.), while VirtualTransport provides simulation capabilities
//! for deterministic load testing and scenario-based verification.

use super::types::{
    TransportHandle, TransportPacket, TransportStats, VirtualRequest, VirtualResponse,
};
use crate::abi::FfiQuicConfig;
use crate::error::LaplaceError;
use async_trait::async_trait;
use std::fmt;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

// ============================================================================
// SovereignTransport (Core Kernel Transport)
// ============================================================================

/// Core abstraction for network transport implementations
///
/// This trait defines the contract between the Laplace kernel (consumer) and
/// transport implementations (providers). It enforces thread safety, error
/// consistency, deterministic lifecycle management, and zero-copy I/O semantics.
///
/// Each transport instance is created with `start()`, returning a handle that
/// remains valid until `stop()` is called. Multiple handles can coexist for
/// simultaneous server instances.
///
/// All methods are async to support the tokio runtime. Implementations must not
/// block or perform CPU-intensive work; I/O operations should use async/await.
/// All errors must be converted to `LaplaceError` variants for uniform kernel
/// error handling.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Transport",
        link = "LEP-0013-laplace-interfaces-pluggable_network_chaos"
    )
)]
#[async_trait]
pub trait SovereignTransport: Send + Sync + fmt::Debug {
    /// Start a transport server instance
    ///
    /// Initializes and binds a server socket, configures TLS/crypto, and begins
    /// accepting incoming client connections. This is a resource allocation point;
    /// failure to stop the server will leak resources.
    ///
    /// # Arguments
    ///
    /// - `config`: Transport configuration (address, port, TLS paths, limits)
    ///
    /// # Returns
    ///
    /// On success, returns an opaque `TransportHandle` (u64) that identifies this
    /// server instance. This handle is used for all subsequent operations on this server.
    ///
    /// # Errors
    ///
    /// - `NetworkError`: Failed to bind to port or socket configuration error
    /// - `InvalidRequest`: Configuration is invalid (port=0, missing TLS certs, etc.)
    /// - `Internal`: Unexpected error (panic recovery, unrecoverable state)
    ///
    /// # Concurrency
    ///
    /// Multiple `start()` calls may be active simultaneously; each produces a
    /// distinct handle. The implementation must manage per-instance state safely.
    async fn start(&self, config: FfiQuicConfig) -> Result<TransportHandle, LaplaceError>;

    /// Stop a running transport server instance
    ///
    /// Gracefully closes all client connections, drains pending packets, and releases
    /// all socket and memory resources associated with this server. After this call,
    /// the handle is invalid.
    ///
    /// # Arguments
    ///
    /// - `handle`: Server instance identifier (from `start()`)
    ///
    /// # Returns
    ///
    /// `Ok(())` on successful shutdown, or `LaplaceError` on failure.
    ///
    /// # Errors
    ///
    /// - `InvalidPointer`: Handle not found (already stopped or never existed)
    /// - `Timeout`: Graceful shutdown exceeded time limit (some resources may leak)
    /// - `Internal`: Unexpected error during shutdown
    ///
    /// # Concurrency
    ///
    /// Safe to call while `dequeue_packet()` or `get_stats()` are running on other tasks.
    /// Ongoing operations may receive errors as the server shuts down.
    async fn stop(&self, handle: TransportHandle) -> Result<(), LaplaceError>;

    /// Dequeue the next received packet from a server instance
    ///
    /// Non-blocking attempt to retrieve a packet from the transport's inbound queue.
    /// Returns immediately whether or not a packet is available. This method is
    /// designed for polling patterns or integration with kernel scheduler loops.
    ///
    /// # Arguments
    ///
    /// - `handle`: Server instance identifier
    ///
    /// # Returns
    ///
    /// - `Ok(Some(packet))`: Packet available; caller owns the packet data
    /// - `Ok(None)`: No packets available (queue empty, not an error)
    /// - `Err(error)`: Server error prevents dequeueing (server stopped, etc.)
    ///
    /// # Errors
    ///
    /// - `InvalidPointer`: Handle not found or server is stopped
    /// - `Internal`: Unexpected queue error
    ///
    /// # Zero-Copy Guarantee
    ///
    /// The returned `TransportPacket.data` (Vec<u8>) is never copied within the
    /// transport layer. The allocation from the network receive path transfers
    /// directly to the caller. Deno can construct a memory view over this buffer
    /// without additional allocation.
    ///
    /// # Concurrency
    ///
    /// Multiple tasks may call `dequeue_packet()` on the same handle. The implementation
    /// must ensure packets are distributed fairly (FIFO) and no packets are lost.
    async fn dequeue_packet(
        &self,
        handle: TransportHandle,
    ) -> Result<Option<TransportPacket>, LaplaceError>;

    /// Retrieve current statistics for a server instance
    ///
    /// Returns a snapshot of performance metrics without blocking. All counter values
    /// are atomic reads; no synchronization or state copying occurs.
    ///
    /// # Arguments
    ///
    /// - `handle`: Server instance identifier
    ///
    /// # Returns
    ///
    /// `Ok(stats)` with current metrics, or `LaplaceError` if the handle is invalid.
    ///
    /// # Errors
    ///
    /// - `InvalidPointer`: Handle not found
    /// - `Internal`: Unexpected error reading metrics
    ///
    /// # Consistency
    ///
    /// The stats snapshot may reflect partially committed updates. For example,
    /// `total_packets_received` might increment between reading two other fields.
    /// The snapshot is not transactional, but each individual counter value is
    /// atomically read at the moment of access.
    async fn get_stats(&self, handle: TransportHandle) -> Result<TransportStats, LaplaceError>;

    /// Enqueue a packet for transmission to a specific client connection
    ///
    /// Asynchronously queues a packet for transmission without blocking on network I/O.
    /// The packet is enqueued and the method returns immediately; actual transmission
    /// happens on the transport's background sender task. Ownership of the packet data
    /// transfers directly to the transport layer, maintaining zero-copy semantics.
    ///
    /// The `connection_id` field in the packet identifies the specific client
    /// connection that should receive the data, enabling the transport to route
    /// packets correctly without additional routing tables.
    ///
    /// # Arguments
    ///
    /// - `handle`: Server instance identifier (from `start()`)
    /// - `packet`: Packet to send, with `connection_id` specifying the recipient
    ///
    /// # Returns
    ///
    /// - `Ok(())`: Packet successfully enqueued for transmission
    /// - `Err(LaplaceError)`: Enqueueing failed
    ///
    /// # Errors
    ///
    /// - `InvalidPointer`: Handle not found or server is stopped
    /// - `QuotaExceeded`: Send queue is full (backpressure limit reached)
    /// - `Internal`: Unexpected error during enqueue
    ///
    /// # Note on Delivery
    ///
    /// This method does NOT guarantee delivery. The packet is queued, but may be
    /// lost if the connection closes before transmission, dropped if the send
    /// queue overflows, or delayed arbitrarily. The kernel is responsible for
    /// implementing retransmission logic if guaranteed delivery is required.
    ///
    /// # Backpressure Handling
    ///
    /// When the send queue reaches its limit, this method returns `QuotaExceeded`.
    /// The kernel should respect this signal and either backoff and retry later,
    /// implement its own queue management upstream, or close the connection if
    /// backpressure persists.
    async fn enqueue_send_packet(
        &self,
        handle: TransportHandle,
        packet: TransportPacket,
    ) -> Result<(), LaplaceError>;

    /// Optional: Query if a server instance is currently running
    ///
    /// Allows the kernel to determine if a handle is valid without attempting
    /// an operation. Default implementation returns `true` (assume running).
    async fn is_running(&self, _handle: TransportHandle) -> bool {
        true
    }
}

/// Factory trait for creating SovereignTransport instances
///
/// Enables dynamic transport selection at runtime. The kernel uses this to
/// instantiate the configured transport backend without compile-time coupling.
pub trait TransportFactory: Send + Sync {
    /// Create a new SovereignTransport instance
    fn create(&self) -> Box<dyn SovereignTransport>;
}

// ============================================================================
// VirtualTransport (Simulation & Testing)
// ============================================================================

/// Virtual transport layer trait for simulation and testing
///
/// This trait provides a seam for request/response simulation, allowing virtual
/// users to send HTTP requests through a simulation layer instead of actual network I/O.
/// Enables deterministic latency injection, response scripting, and failure scenario testing.
///
/// The trait is designed to be object-safe, enabling `Box<dyn VirtualTransport>`
/// for heterogeneous transport implementations (mock, latency-injected, script-driven, etc.).
pub trait VirtualTransport: Send + Sync {
    /// Send a virtual request and receive a virtual response
    ///
    /// Handles a complete request-response cycle within the simulation layer.
    /// The implementation processes the request and generates or looks up a
    /// response based on request properties, applying appropriate latency
    /// simulation before returning.
    ///
    /// # Arguments
    ///
    /// - `req`: The virtual request to send
    ///
    /// # Returns
    ///
    /// - `Ok(response)`: The response from the virtual network layer
    /// - `Err(LaplaceError::NetworkError)`: Network-level error (timeout, connection refused, etc.)
    /// - `Err(other)`: Other error types
    ///
    /// # Implementation Notes
    ///
    /// Implementations should validate the request (method, path, headers, body),
    /// simulate network processing including latency, and generate or lookup a
    /// response based on request properties. The response should have appropriate
    /// `latency_ticks` set for accurate simulation timing.
    ///
    /// For reproducible simulation, implementations should base responses on
    /// deterministic criteria (request properties, VU ID, tick timestamp) rather
    /// than randomness or wall-clock time.
    fn send_request(
        &self,
        req: VirtualRequest,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<VirtualResponse, LaplaceError>> + Send + '_>,
    >;
}
