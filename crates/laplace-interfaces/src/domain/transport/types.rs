//! # Transport Data Types
//!
//! Defines all data structures, enumerations, and opaque handles used by
//! transport layer traits. Includes packet metadata, statistics, HTTP semantics,
//! and deterministic seed management for virtual user execution.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

// ============================================================================
// QUIC Transport Types (always available)
// ============================================================================

/// Unique identifier for a transport server instance
///
/// Opaque 64-bit handle managed by the transport implementation.
/// Valid only for the lifetime of the server instance; becomes invalid after `stop()`.
pub type TransportHandle = u64;

/// Packet metadata and zero-copy buffer reference
///
/// Produced by the transport layer and consumed by the kernel.
/// The contained buffer remains valid only while pinned in the queue.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Transport",
        link = "LEP-0013-laplace-interfaces-pluggable_network_chaos"
    )
)]
#[derive(Debug, Clone)]
pub struct TransportPacket {
    /// Raw packet bytes (pointer owned by transport, pinned in queue)
    pub data: Vec<u8>,

    /// Source connection identifier (connection-scoped within this server)
    pub connection_id: u64,

    /// Timestamp of packet receipt (microseconds since Unix epoch)
    pub timestamp_us: u64,

    /// Protocol-specific stream identifier (optional, transport-dependent)
    pub stream_id: Option<u64>,
}

impl TransportPacket {
    /// Create a new transport packet with current timestamp
    pub fn new(data: Vec<u8>, connection_id: u64) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);

        Self {
            data,
            connection_id,
            timestamp_us: now,
            stream_id: None,
        }
    }

    /// Byte length of packet data
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Check if packet is empty
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Pointer to packet data (for FFI and Deno memory view compatibility)
    pub fn as_ptr(&self) -> *const u8 {
        self.data.as_ptr()
    }
}

/// Transport layer statistics snapshot
///
/// Provides real-time metrics for monitoring and resource decision-making.
/// All counters are atomic snapshots—no blocking or coordination required.
#[derive(Debug, Clone, Default)]
pub struct TransportStats {
    /// Total incoming packets processed since server start
    pub total_packets_received: u64,

    /// Currently active client connections
    pub active_connections: u32,

    /// Total bytes ingress (client → server)
    pub total_bytes_in: u64,

    /// Total bytes egress (server → client)
    pub total_bytes_out: u64,

    /// Cumulative protocol errors (malformed packets, timeout, etc.)
    pub error_count: u64,

    /// Server uptime in milliseconds
    pub uptime_ms: u64,

    /// Average latency per packet in milliseconds (uptime / packets)
    pub avg_latency_ms: f64,
}

// ============================================================================
// HTTP Virtual Transport Types (feature = "twin")
// ============================================================================

/// HTTP request methods
///
/// Represents standard HTTP verb operations that virtual users can execute.
/// Extended in future phases to support custom methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum HttpMethod {
    /// HTTP GET: retrieve resource
    Get,
    /// HTTP POST: create or submit resource
    Post,
    /// HTTP PUT: replace entire resource
    Put,
    /// HTTP DELETE: remove resource
    Delete,
    /// HTTP PATCH: partial update to resource
    Patch,
    /// HTTP HEAD: retrieve headers only
    Head,
    /// HTTP OPTIONS: request communication options
    Options,
}

impl fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HttpMethod::Get => write!(f, "GET"),
            HttpMethod::Post => write!(f, "POST"),
            HttpMethod::Put => write!(f, "PUT"),
            HttpMethod::Delete => write!(f, "DELETE"),
            HttpMethod::Patch => write!(f, "PATCH"),
            HttpMethod::Head => write!(f, "HEAD"),
            HttpMethod::Options => write!(f, "OPTIONS"),
        }
    }
}

impl HttpMethod {
    /// Check if this method is idempotent (safe to retry)
    pub fn is_idempotent(self) -> bool {
        matches!(
            self,
            HttpMethod::Get | HttpMethod::Head | HttpMethod::Put | HttpMethod::Delete
        )
    }

    /// Check if this method is safe (read-only, no side effects)
    pub fn is_safe(self) -> bool {
        matches!(
            self,
            HttpMethod::Get | HttpMethod::Head | HttpMethod::Options
        )
    }
}

/// Virtual HTTP request sent by a virtual user
///
/// Contains all necessary information for a virtual user to send an HTTP request
/// through the simulation layer instead of actual network I/O.
#[derive(Debug, Clone)]
pub struct VirtualRequest {
    /// Virtual User identifier sending this request
    pub vu_id: VUID,

    /// HTTP method for this request
    pub method: HttpMethod,

    /// URL path for the request
    pub path: String,

    /// HTTP headers as key-value pairs
    pub headers: HashMap<String, String>,

    /// Optional request body
    pub body: Option<Vec<u8>>,

    /// Simulation tick timestamp when this request was sent
    pub sent_at_tick: u64,
}

impl VirtualRequest {
    /// Create a new virtual request
    pub fn new(
        vu_id: VUID,
        method: HttpMethod,
        path: impl Into<String>,
        sent_at_tick: u64,
    ) -> Self {
        Self {
            vu_id,
            method,
            path: path.into(),
            headers: HashMap::new(),
            body: None,
            sent_at_tick,
        }
    }

    /// Set the request body
    pub fn with_body(mut self, body: Vec<u8>) -> Self {
        self.body = Some(body);
        self
    }

    /// Set a header
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Get a header value (case-sensitive)
    pub fn get_header(&self, key: &str) -> Option<&String> {
        self.headers.get(key)
    }
}

/// Virtual HTTP response received from the network layer
///
/// Contains response metadata and payload for virtual user processing.
#[derive(Debug, Clone)]
pub struct VirtualResponse {
    /// HTTP status code
    pub status: u16,

    /// HTTP response headers as key-value pairs
    pub headers: HashMap<String, String>,

    /// Optional response body
    pub body: Option<Vec<u8>>,

    /// Simulated network latency in simulation ticks
    pub latency_ticks: u64,
}

impl VirtualResponse {
    /// Create a new virtual response
    pub fn new(status: u16, latency_ticks: u64) -> Self {
        Self {
            status,
            headers: HashMap::new(),
            body: None,
            latency_ticks,
        }
    }

    /// Create a successful response (HTTP 200 OK)
    pub fn ok(latency_ticks: u64) -> Self {
        Self::new(200, latency_ticks)
    }

    /// Create a created response (HTTP 201 Created)
    pub fn created(latency_ticks: u64) -> Self {
        Self::new(201, latency_ticks)
    }

    /// Create a bad request response (HTTP 400)
    pub fn bad_request(latency_ticks: u64) -> Self {
        Self::new(400, latency_ticks)
    }

    /// Create a not found response (HTTP 404)
    pub fn not_found(latency_ticks: u64) -> Self {
        Self::new(404, latency_ticks)
    }

    /// Create a server error response (HTTP 500)
    pub fn server_error(latency_ticks: u64) -> Self {
        Self::new(500, latency_ticks)
    }

    /// Set the response body
    pub fn with_body(mut self, body: Vec<u8>) -> Self {
        self.body = Some(body);
        self
    }

    /// Set a response header
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Get a header value (case-sensitive)
    pub fn get_header(&self, key: &str) -> Option<&String> {
        self.headers.get(key)
    }

    /// Check if the response indicates success (status 2xx)
    pub fn is_success(&self) -> bool {
        self.status >= 200 && self.status < 300
    }

    /// Check if the response indicates client error (status 4xx)
    pub fn is_client_error(&self) -> bool {
        self.status >= 400 && self.status < 500
    }

    /// Check if the response indicates server error (status 5xx)
    pub fn is_server_error(&self) -> bool {
        self.status >= 500 && self.status < 600
    }
}

// ============================================================================
// Kraken DNA Types (feature = "twin")
// ============================================================================

/// Virtual User Identifier
///
/// Corresponds to `vu_id` in KrakenDNA.tla and `VUID` type.
/// Values range from 1 to MaxVUs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VUID(pub u64);

impl VUID {
    /// Create a new VUID
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    /// Get the inner value
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for VUID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VU#{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_packet_creation() {
        let data = vec![1, 2, 3, 4, 5];
        let ptr_before = data.as_ptr();
        let packet = TransportPacket::new(data, 42);

        assert_eq!(packet.connection_id, 42);
        assert_eq!(packet.len(), 5);
        assert_eq!(packet.as_ptr(), ptr_before);
        assert!(!packet.is_empty());
    }

    #[test]
    fn transport_stats_default() {
        let stats = TransportStats::default();
        assert_eq!(stats.total_packets_received, 0);
        assert_eq!(stats.active_connections, 0);
        assert_eq!(stats.error_count, 0);
    }

    #[test]
    fn http_method_display() {
        assert_eq!(HttpMethod::Get.to_string(), "GET");
        assert_eq!(HttpMethod::Post.to_string(), "POST");
    }

    #[test]
    fn http_method_idempotent() {
        assert!(HttpMethod::Get.is_idempotent());
        assert!(!HttpMethod::Post.is_idempotent());
    }

    #[test]
    fn virtual_request_creation() {
        let vu_id = VUID::new(1);
        let request = VirtualRequest::new(vu_id, HttpMethod::Get, "/api/users", 100);
        assert_eq!(request.vu_id, vu_id);
        assert_eq!(request.method, HttpMethod::Get);
    }

    #[test]
    fn virtual_response_ok() {
        let response = VirtualResponse::ok(50);
        assert!(response.is_success());
    }

    #[test]
    fn vuid_creation_and_display() {
        let vu = VUID::new(42);
        assert_eq!(vu.as_u64(), 42);
        assert_eq!(vu.to_string(), "VU#42");
    }
}
