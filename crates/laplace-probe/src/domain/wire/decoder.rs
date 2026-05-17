// SPDX-License-Identifier: Apache-2.0
//! Layer 1 semantic decoder: LEB128 VarInt ID + payload → endpoint string.
//!
//! ## Frame layout expected
//! ```text
//! [LEB128 varint: route_id (u32)]  ← 1–5 bytes
//! [N bytes: payload]
//! ```

use super::dictionary::{read_varint, StaticDictionary};
use super::error::MeshError;

/// Decodes binary frames back into endpoint strings and raw payloads.
///
/// The decode path is **zero-copy**: the returned `&str` is a reference into
/// the `StaticDictionary`'s internal `Vec<String>`, and the payload slice
/// points into the original `frame` — no heap allocations occur.
pub struct SemanticDecoder;

impl SemanticDecoder {
    /// Decode a binary frame into `(endpoint, payload)`.
    ///
    /// # Errors
    /// - [`MeshError::InvalidFrame`] if `frame` is empty or the varint is truncated.
    /// - [`MeshError::UnknownId`] if the decoded ID has no entry in `dict`.
    pub fn decode<'a>(
        dict: &'a StaticDictionary,
        frame: &'a [u8],
    ) -> Result<(&'a str, &'a [u8]), MeshError> {
        if frame.is_empty() {
            return Err(MeshError::InvalidFrame);
        }

        let (id, consumed) = read_varint(frame).ok_or(MeshError::InvalidFrame)?;
        let endpoint = dict.get_path(id).ok_or(MeshError::UnknownId(id))?;
        let payload = &frame[consumed..];
        Ok((endpoint, payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::wire::{dictionary::StaticDictionary, encoder::SemanticEncoder};

    fn make_dict() -> StaticDictionary {
        let mut d = StaticDictionary::new();
        d.insert("GET /api/users".to_string()).unwrap();
        d.insert("POST /api/orders".to_string()).unwrap();
        d
    }

    #[test]
    fn test_decode_success() {
        let dict = make_dict();
        let frame = [0x01u8, b'h', b'i'];
        let (endpoint, payload) = SemanticDecoder::decode(&dict, &frame).unwrap();

        assert_eq!(endpoint, "GET /api/users");
        assert_eq!(payload, b"hi");
    }

    #[test]
    fn test_decode_empty_payload() {
        let dict = make_dict();
        let frame = [0x02u8];
        let (endpoint, payload) = SemanticDecoder::decode(&dict, &frame).unwrap();

        assert_eq!(endpoint, "POST /api/orders");
        assert_eq!(payload, b"");
    }

    #[test]
    fn test_decode_empty_frame() {
        let dict = make_dict();
        let err = SemanticDecoder::decode(&dict, &[]).unwrap_err();

        assert!(matches!(err, MeshError::InvalidFrame));
    }

    #[test]
    fn test_decode_unknown_id() {
        let dict = make_dict();
        // ID 0xFF = 255, encodes as two LEB128 bytes [0xFF, 0x01]
        let frame = [0xFFu8, 0x01, 0x00];
        let err = SemanticDecoder::decode(&dict, &frame).unwrap_err();

        assert!(matches!(err, MeshError::UnknownId(255)));
    }

    #[test]
    fn test_roundtrip_with_binary_payload() {
        let dict = make_dict();
        let endpoint = "GET /api/users";
        let payload = &[0xDE, 0xAD, 0xBE, 0xEF];

        let frame = SemanticEncoder::encode(&dict, endpoint, payload).unwrap();
        let (decoded_endpoint, decoded_payload) = SemanticDecoder::decode(&dict, &frame).unwrap();

        assert_eq!(decoded_endpoint, endpoint);
        assert_eq!(decoded_payload, payload);
    }

    #[test]
    fn test_roundtrip_empty_payload() {
        let dict = make_dict();
        let endpoint = "POST /api/orders";

        let frame = SemanticEncoder::encode(&dict, endpoint, b"").unwrap();
        let (decoded_endpoint, decoded_payload) = SemanticDecoder::decode(&dict, &frame).unwrap();

        assert_eq!(decoded_endpoint, endpoint);
        assert_eq!(decoded_payload, b"");
    }

    #[test]
    fn test_roundtrip_all_entries() {
        let mut dict = StaticDictionary::new();
        let endpoints = ["GET /a", "POST /b", "DELETE /c", "PUT /d"];
        for ep in &endpoints {
            dict.insert(ep.to_string()).unwrap();
        }

        for ep in &endpoints {
            let payload = ep.as_bytes();
            let frame = SemanticEncoder::encode(&dict, ep, payload).unwrap();
            let (decoded_ep, decoded_payload) = SemanticDecoder::decode(&dict, &frame).unwrap();
            assert_eq!(decoded_ep, *ep);
            assert_eq!(decoded_payload, payload);
        }
    }

    #[test]
    fn test_roundtrip_high_id() {
        let mut dict = StaticDictionary::new();
        for i in 0u32..128 {
            dict.insert(format!("GET /ep/{i}")).unwrap();
        }
        let ep = "GET /ep/127";
        let frame = SemanticEncoder::encode(&dict, ep, b"data").unwrap();
        let (decoded, payload) = SemanticDecoder::decode(&dict, &frame).unwrap();
        assert_eq!(decoded, ep);
        assert_eq!(payload, b"data");
    }
}
