use crate::domain::TransportStats;
use serde::{Deserialize, Serialize};

/// Internal statistics snapshot (mapped to TransportStats)
///
/// Maintains the domain-specific representation before conversion
/// to the laplace-core transport trait layer.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QuicServerStats {
    /// Total number of requests processed since the server started.
    pub total_requests: u64,
    /// Current number of active client connections.
    pub active_connections: u32,
    /// Cumulative volume of data received by the server (ingress).
    pub total_bytes_in: u64,
    /// Cumulative volume of data sent by the server (egress).
    pub total_bytes_out: u64,
    /// Average request-response latency measured in milliseconds.
    pub avg_latency_ms: f64,
    /// Total number of failed operations or errors encountered.
    pub error_count: u64,
    /// Total duration the server has been operational in milliseconds.
    pub uptime_ms: u64,
}

impl QuicServerStats {
    /// Convert to trait-level TransportStats
    pub fn into_transport_stats(self) -> TransportStats {
        TransportStats {
            total_packets_received: self.total_requests,
            active_connections: self.active_connections,
            total_bytes_in: self.total_bytes_in,
            total_bytes_out: self.total_bytes_out,
            error_count: self.error_count,
            uptime_ms: self.uptime_ms,
            avg_latency_ms: self.avg_latency_ms,
        }
    }

    /// Serialize statistics to JSON bytes
    pub fn to_json_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        let json_str = serde_json::to_string(&self)?;
        Ok(json_str.into_bytes())
    }

    /// Deserialize statistics from JSON bytes
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        let json_str = String::from_utf8_lossy(bytes);
        serde_json::from_str(&json_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quic_server_stats_creation() {
        let stats = QuicServerStats {
            total_requests: 100,
            active_connections: 5,
            total_bytes_in: 1000,
            total_bytes_out: 500,
            avg_latency_ms: 10.5,
            error_count: 2,
            uptime_ms: 60000,
        };

        assert_eq!(stats.total_requests, 100);
        assert_eq!(stats.active_connections, 5);
    }

    #[test]
    fn quic_server_stats_conversion() {
        let stats = QuicServerStats {
            total_requests: 50,
            active_connections: 3,
            total_bytes_in: 500,
            total_bytes_out: 250,
            avg_latency_ms: 5.0,
            error_count: 1,
            uptime_ms: 30000,
        };

        let transport_stats = stats.into_transport_stats();
        assert_eq!(transport_stats.total_packets_received, 50);
        assert_eq!(transport_stats.active_connections, 3);
    }

    #[test]
    fn quic_server_stats_serialization() {
        let stats = QuicServerStats {
            total_requests: 100,
            active_connections: 5,
            total_bytes_in: 1000,
            total_bytes_out: 500,
            avg_latency_ms: 10.5,
            error_count: 2,
            uptime_ms: 60000,
        };

        let json_bytes = stats.to_json_bytes().expect("serialization failed");
        let deserialized =
            QuicServerStats::from_json_bytes(&json_bytes).expect("deserialization failed");

        assert_eq!(deserialized.total_requests, 100);
        assert_eq!(deserialized.active_connections, 5);
    }
}
