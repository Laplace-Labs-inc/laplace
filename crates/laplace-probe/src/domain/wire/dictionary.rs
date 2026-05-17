// SPDX-License-Identifier: Apache-2.0
//! Layer 1: Static dictionary with LEB128 VarInt IDs.
//!
//! ## ID ranges
//!
//! | Range           | Use                                    |
//! |-----------------|----------------------------------------|
//! | `0x0001–0x3FFF` | Static dictionary (this module)        |
//! | `0x4000–0x7FFF` | Reserved                               |
//! | `0x8000–0xFFFF` | Dynamic session dictionary (Layer 2)   |

use super::error::MeshError;
use std::collections::HashMap;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

// ── ID range constants ────────────────────────────────────────────────────────

/// First valid static dictionary ID.
pub const STATIC_ID_MIN: u32 = 0x0001;
/// Last valid static dictionary ID (16 383 entries maximum).
pub const STATIC_ID_MAX: u32 = 0x3FFF;

// ── LEB128 VarInt helpers (unsigned) ─────────────────────────────────────────

/// Encode a `u32` as an unsigned LEB128 sequence and append to `buf`.
///
/// - IDs 1–127 encode as **1 byte**
/// - IDs 128–16383 encode as **2 bytes**
/// - IDs 16384–2097151 encode as **3 bytes**
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "40_Probe_Wire",
        link = "LEP-0014-laplace-probe-wire_protocol_and_compression"
    )
)]
pub fn write_varint(mut value: u32, buf: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80; // continuation bit
        }
        buf.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// Decode an unsigned LEB128 `u32` from the start of `data`.
///
/// Returns `(value, bytes_consumed)` on success, or `None` on truncated /
/// overflow input.
pub fn read_varint(data: &[u8]) -> Option<(u32, usize)> {
    let mut result: u32 = 0;
    let mut shift: u32 = 0;
    for (i, &byte) in data.iter().enumerate() {
        if shift >= 32 {
            return None; // overflow
        }
        result |= ((byte & 0x7F) as u32) << shift;
        if byte & 0x80 == 0 {
            return Some((result, i + 1));
        }
        shift += 7;
    }
    None // truncated
}

// ── StaticDictionary ─────────────────────────────────────────────────────────

/// Static dictionary mapping endpoint strings to LEB128-encoded u32 IDs.
///
/// IDs are assigned in the range `STATIC_ID_MIN..=STATIC_ID_MAX` (1–16 383).
///
/// ## Zero-allocation decode
///
/// `get_path(id)` returns a `&str` borrowed directly from the internal
/// `Vec<String>` — no heap allocation on the hot decode path.
///
/// ## Thread safety
///
/// `StaticDictionary` is `Send + Sync`; all mutations must complete before
/// sharing across threads.
#[derive(Debug, Clone)]
pub struct StaticDictionary {
    /// Forward lookup: `entries[id - 1]` → endpoint string.
    /// IDs start at 1, so index = id − 1.
    entries: Vec<String>,
    /// Reverse lookup: endpoint string → u32 ID.
    reverse_map: HashMap<String, u32>,
}

impl StaticDictionary {
    /// Create a new empty dictionary.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            reverse_map: HashMap::new(),
        }
    }

    /// Insert `path` and return its assigned u32 ID.
    ///
    /// If `path` was already present the existing ID is returned without
    /// reinsertion (idempotent).
    ///
    /// # Errors
    /// - [`MeshError::CapacityExceeded`] if the dictionary is full
    ///   (`STATIC_ID_MAX` entries already inserted).
    pub fn insert(&mut self, path: String) -> Result<u32, MeshError> {
        if let Some(&id) = self.reverse_map.get(&path) {
            return Ok(id);
        }

        if self.entries.len() >= (STATIC_ID_MAX - STATIC_ID_MIN + 1) as usize {
            return Err(MeshError::CapacityExceeded);
        }

        let id = self.entries.len() as u32 + STATIC_ID_MIN;
        self.reverse_map.insert(path.clone(), id);
        self.entries.push(path);
        Ok(id)
    }

    /// Look up the u32 ID for an endpoint string (encode direction).
    pub fn get_id(&self, path: &str) -> Option<u32> {
        self.reverse_map.get(path).copied()
    }

    /// Look up the endpoint string for a u32 ID (decode direction).
    ///
    /// Returns a `&str` borrowed from internal storage — zero allocation.
    pub fn get_path(&self, id: u32) -> Option<&str> {
        let idx = id.checked_sub(STATIC_ID_MIN)? as usize;
        self.entries.get(idx).map(String::as_str)
    }

    /// Number of entries currently in the dictionary.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if the dictionary contains no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for StaticDictionary {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_assigns_sequential_ids_from_one() {
        let mut d = StaticDictionary::new();
        assert_eq!(d.insert("GET /a".into()).unwrap(), 1);
        assert_eq!(d.insert("POST /b".into()).unwrap(), 2);
        assert_eq!(d.insert("DELETE /c".into()).unwrap(), 3);
    }

    #[test]
    fn insert_idempotent() {
        let mut d = StaticDictionary::new();
        let id1 = d.insert("GET /a".into()).unwrap();
        let id2 = d.insert("GET /a".into()).unwrap();
        assert_eq!(id1, id2);
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn get_path_zero_alloc() {
        let mut d = StaticDictionary::new();
        d.insert("GET /api/users".into()).unwrap();
        d.insert("POST /api/orders".into()).unwrap();
        assert_eq!(d.get_path(1), Some("GET /api/users"));
        assert_eq!(d.get_path(2), Some("POST /api/orders"));
        assert!(d.get_path(0).is_none());
        assert!(d.get_path(999).is_none());
    }

    #[test]
    fn varint_roundtrip_single_byte() {
        for v in [0u32, 1, 63, 127] {
            let mut buf = Vec::new();
            write_varint(v, &mut buf);
            assert_eq!(buf.len(), 1);
            let (decoded, consumed) = read_varint(&buf).unwrap();
            assert_eq!(decoded, v);
            assert_eq!(consumed, 1);
        }
    }

    #[test]
    fn varint_roundtrip_two_bytes() {
        for v in [128u32, 255, 16383] {
            let mut buf = Vec::new();
            write_varint(v, &mut buf);
            assert_eq!(buf.len(), 2, "expected 2 bytes for {v}");
            let (decoded, consumed) = read_varint(&buf).unwrap();
            assert_eq!(decoded, v);
            assert_eq!(consumed, 2);
        }
    }

    #[test]
    fn varint_roundtrip_three_bytes() {
        let v: u32 = 0x8000;
        let mut buf = Vec::new();
        write_varint(v, &mut buf);
        assert_eq!(buf.len(), 3);
        let (decoded, consumed) = read_varint(&buf).unwrap();
        assert_eq!(decoded, v);
        assert_eq!(consumed, 3);
    }

    #[test]
    fn varint_partial_returns_none() {
        // Only continuation bytes — truncated
        assert!(read_varint(&[0x80, 0x80]).is_none());
    }
}
