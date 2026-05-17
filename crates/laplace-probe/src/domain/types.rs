// SPDX-License-Identifier: Apache-2.0
//! Domain Types Module
//!
//! Core domain data structures for packet handling and statistics.
//! Includes trait conversions for bridging to laplace-core abstractions.

use laplace_interfaces::TransportPacket;
use std::time::{SystemTime, UNIX_EPOCH};

/// Internal packet representation before conversion to TransportPacket
///
/// This maintains zero-copy semantics during queue transit by preserving
/// the original Vec<u8> allocation from the network receive path.
#[derive(Debug, Clone)]
pub struct PacketBuffer {
    /// Raw packet bytes (pinned in queue)
    pub data: Vec<u8>,
    /// Source connection handle
    pub connection_handle: u64,
    /// Timestamp of receipt (microseconds since epoch)
    pub timestamp_us: u64,
    /// Stream ID if applicable
    pub stream_id: Option<u64>,
}

impl PacketBuffer {
    /// Create a new packet buffer with current timestamp
    pub fn new(data: Vec<u8>, connection_handle: u64) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);

        Self {
            data,
            connection_handle,
            timestamp_us: now,
            stream_id: None,
        }
    }

    /// Convert to trait-level TransportPacket (zero-copy: same Vec ownership)
    pub fn into_transport_packet(self) -> TransportPacket {
        TransportPacket {
            data: self.data,
            connection_id: self.connection_handle,
            timestamp_us: self.timestamp_us,
            stream_id: self.stream_id,
        }
    }

    /// Size in bytes
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Pointer to data (for FFI access)
    pub fn as_ptr(&self) -> *const u8 {
        self.data.as_ptr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_buffer_creation() {
        let data = vec![1, 2, 3, 4, 5];
        let expected_ptr = data.as_ptr();
        let expected_len = data.len();

        let packet = PacketBuffer::new(data, 42);

        assert_eq!(packet.connection_handle, 42);
        assert_eq!(packet.len(), 5);
        assert_eq!(packet.len(), expected_len);
        assert_eq!(
            packet.as_ptr(),
            expected_ptr,
            "PacketBuffer must preserve original Vec allocation (zero-copy requirement)"
        );
    }

    #[test]
    fn packet_buffer_conversion() {
        let packet = PacketBuffer::new(vec![1, 2, 3], 10);
        let transport_packet = packet.into_transport_packet();

        assert_eq!(transport_packet.connection_id, 10);
        assert_eq!(transport_packet.len(), 3);
    }
}
