//! Layer 1 semantic encoder: endpoint string → LEB128 VarInt ID + payload.
//!
//! ## Frame layout
//! ```text
//! [LEB128 varint: route_id (u32)]  ← 1–5 bytes
//! [N bytes: payload]
//! ```

use super::dictionary::{write_varint, StaticDictionary};
use super::error::MeshError;

/// Encodes HTTP endpoint strings into compact binary frames.
///
/// Frame layout: `[LEB128 VarInt route_id][payload bytes]`
///
/// IDs in the range `0x0001–0x3FFF` encode as 1–2 bytes (LEB128), giving
/// significant savings over the previous single-byte scheme (max 254 entries).
pub struct SemanticEncoder;

impl SemanticEncoder {
    /// Encode an endpoint + payload into a binary frame.
    ///
    /// # Errors
    /// - [`MeshError::EndpointNotFound`] if `endpoint` is not in `dict`.
    pub fn encode(
        dict: &StaticDictionary,
        endpoint: &str,
        payload: &[u8],
    ) -> Result<Vec<u8>, MeshError> {
        let id = dict
            .get_id(endpoint)
            .ok_or_else(|| MeshError::EndpointNotFound(endpoint.to_string()))?;

        // Encode ID as LEB128 varint (1–3 bytes for the 0x0001–0x3FFF range)
        let mut frame = Vec::with_capacity(3 + payload.len());
        write_varint(id, &mut frame);
        frame.extend_from_slice(payload);
        Ok(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::wire::dictionary::StaticDictionary;

    fn make_dict() -> StaticDictionary {
        let mut d = StaticDictionary::new();
        d.insert("GET /api/users".to_string()).unwrap();
        d.insert("POST /api/orders".to_string()).unwrap();
        d
    }

    #[test]
    fn test_encode_success() {
        let dict = make_dict();
        let payload = b"hello";
        let frame = SemanticEncoder::encode(&dict, "GET /api/users", payload).unwrap();

        // ID 1 encodes as a single byte 0x01 in LEB128
        assert_eq!(frame[0], 0x01);
        assert_eq!(&frame[1..], payload);
    }

    #[test]
    fn test_encode_second_entry() {
        let dict = make_dict();
        let frame = SemanticEncoder::encode(&dict, "POST /api/orders", b"data").unwrap();

        // ID 2 encodes as a single byte 0x02 in LEB128
        assert_eq!(frame[0], 0x02);
        assert_eq!(&frame[1..], b"data");
    }

    #[test]
    fn test_encode_empty_payload() {
        let dict = make_dict();
        let frame = SemanticEncoder::encode(&dict, "GET /api/users", b"").unwrap();

        // Only the varint ID byte, no payload
        assert_eq!(frame.len(), 1);
        assert_eq!(frame[0], 0x01);
    }

    #[test]
    fn test_encode_unknown_endpoint() {
        let dict = make_dict();
        let err = SemanticEncoder::encode(&dict, "DELETE /api/ghost", b"").unwrap_err();

        assert!(matches!(err, MeshError::EndpointNotFound(s) if s == "DELETE /api/ghost"));
    }

    #[test]
    fn test_encode_binary_payload() {
        let dict = make_dict();
        let payload = &[0x00, 0xFF, 0xAB, 0xCD];
        let frame = SemanticEncoder::encode(&dict, "GET /api/users", payload).unwrap();

        assert_eq!(&frame[1..], payload);
    }

    #[test]
    fn test_encode_high_id_uses_two_varint_bytes() {
        // Insert 128 entries; the 128th gets ID 128 (0x80) which encodes as 2 LEB128 bytes
        let mut dict = StaticDictionary::new();
        for i in 0u32..128 {
            dict.insert(format!("GET /ep/{i}")).unwrap();
        }
        let frame = SemanticEncoder::encode(&dict, "GET /ep/127", &[]).unwrap();
        // ID 128 in LEB128 = [0x80, 0x01]
        assert_eq!(frame.len(), 2);
        assert_eq!(frame[0], 0x80);
        assert_eq!(frame[1], 0x01);
    }
}
