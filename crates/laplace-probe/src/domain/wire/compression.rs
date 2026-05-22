// SPDX-License-Identifier: Apache-2.0
//! Layer 2: Dynamic session dictionary and DictSync protocol.
//!
//! ## Overview
//!
//! Layer 2 complements the static dictionary (Layer 1) with a per-connection
//! dynamic dictionary.  Tokens that appear frequently enough during a session
//! are **promoted** to a compact VarInt ID, saving bandwidth for repeated
//! strings that are not in the pre-built static dictionary.
//!
//! ## Dynamic ID range
//!
//! Dynamic IDs live in `0x8000–0xFFFF` (32 768 slots).
//!
//! ## Promotion threshold
//!
//! A token is proposed for promotion once it has been observed
//! `PROMOTION_THRESHOLD` times by the `TokenFrequencyTracker`.
//!
//! ## DictSync wire protocol (binary control messages on stream 0)
//!
//! ```text
//! PROPOSE  = 0x01  [varint: proposed_id][u16 BE: token_len][token bytes]
//! ACK      = 0x02  [varint: confirmed_id]
//! REJECT   = 0x03  [varint: rejected_id]
//! ```
//!
//! **Note**: The stream-level sync logic is stubbed here. The data structures
//! are complete and ready for integration in Phase 4.

use std::collections::{HashMap, HashSet};

use super::dictionary::{read_varint, write_varint};

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Minimum observation count before a token is eligible for promotion.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "40_Probe_Wire",
        link = "LEP-0014-laplace-probe-wire_protocol_and_compression"
    )
)]
pub const PROMOTION_THRESHOLD: u32 = 16;

/// First dynamic dictionary ID.
pub const DYNAMIC_ID_MIN: u32 = 0x8000;
/// Last dynamic dictionary ID.
pub const DYNAMIC_ID_MAX: u32 = 0xFFFF;

// ── DictSync message types ────────────────────────────────────────────────────

/// Message type discriminant byte for the DictSync protocol.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DictSyncType {
    Propose = 0x01,
    Ack = 0x02,
    Reject = 0x03,
}

/// A DictSync control message exchanged on stream 0 to negotiate dynamic
/// dictionary additions between peers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DictSyncMessage {
    /// Sender proposes adding `token` with the given ID.
    Propose { proposed_id: u32, token: String },
    /// Receiver accepts the proposed ID mapping.
    Ack { confirmed_id: u32 },
    /// Receiver rejects the proposed ID mapping (e.g. ID conflict).
    Reject { rejected_id: u32 },
}

/// DictSync negotiation failure.
///
/// Semantic decoders must reject dynamic IDs that were rejected or never
/// confirmed, preventing silent decode corruption.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum DictSyncError {
    #[error("dynamic ID {0:#06x} was rejected; using it would silently produce wrong output")]
    RejectedId(u32),
    #[error("dynamic ID {0:#06x} is unknown (Propose not received or Ack not confirmed)")]
    UnknownId(u32),
}

/// Per-session DictSync negotiation state.
///
/// Stream 0 feeds received control messages here. Decoders must call
/// [`Self::check_id`] before using any dynamic ID.
#[derive(Debug, Default)]
pub struct DictSyncSession {
    confirmed: HashMap<u32, String>,
    rejected: HashSet<u32>,
    pending: HashMap<u32, String>,
}

impl DictSyncSession {
    /// Creates an empty DictSync session.
    pub fn new() -> Self {
        Self::default()
    }

    /// Handles a DictSync control message.
    pub fn handle(&mut self, msg: DictSyncMessage) {
        match msg {
            DictSyncMessage::Propose { proposed_id, token } => {
                self.pending.insert(proposed_id, token);
                self.rejected.remove(&proposed_id);
            }
            DictSyncMessage::Ack { confirmed_id } => {
                if let Some(token) = self.pending.remove(&confirmed_id) {
                    self.confirmed.insert(confirmed_id, token);
                    self.rejected.remove(&confirmed_id);
                }
            }
            DictSyncMessage::Reject { rejected_id } => {
                self.pending.remove(&rejected_id);
                self.confirmed.remove(&rejected_id);
                self.rejected.insert(rejected_id);
            }
        }
    }

    /// Registers a locally proposed ID while waiting for Ack/Reject.
    pub fn register_pending(&mut self, id: u32, token: String) {
        self.pending.insert(id, token);
        self.rejected.remove(&id);
    }

    /// Checks whether a dynamic ID is confirmed and returns its token.
    pub fn check_id(&self, id: u32) -> Result<&str, DictSyncError> {
        #[allow(clippy::manual_range_contains)]
        if id < DYNAMIC_ID_MIN || id > DYNAMIC_ID_MAX {
            return Err(DictSyncError::UnknownId(id));
        }
        if self.rejected.contains(&id) {
            return Err(DictSyncError::RejectedId(id));
        }
        self.confirmed
            .get(&id)
            .map(String::as_str)
            .ok_or(DictSyncError::UnknownId(id))
    }
}

impl DictSyncMessage {
    /// Serialize to bytes for transmission.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        match self {
            Self::Propose { proposed_id, token } => {
                buf.push(DictSyncType::Propose as u8);
                write_varint(*proposed_id, &mut buf);
                let token_bytes = token.as_bytes();
                let len = token_bytes.len() as u16;
                buf.extend_from_slice(&len.to_be_bytes());
                buf.extend_from_slice(token_bytes);
            }
            Self::Ack { confirmed_id } => {
                buf.push(DictSyncType::Ack as u8);
                write_varint(*confirmed_id, &mut buf);
            }
            Self::Reject { rejected_id } => {
                buf.push(DictSyncType::Reject as u8);
                write_varint(*rejected_id, &mut buf);
            }
        }
        buf
    }

    /// Deserialize from bytes. Returns `(message, bytes_consumed)` on success.
    pub fn from_bytes(data: &[u8]) -> Option<(Self, usize)> {
        let msg_type = *data.first()?;
        let rest = &data[1..];

        match msg_type {
            0x01 => {
                // PROPOSE
                let (id, id_len) = read_varint(rest)?;
                let rest = &rest[id_len..];
                if rest.len() < 2 {
                    return None;
                }
                let token_len = u16::from_be_bytes([rest[0], rest[1]]) as usize;
                let rest = &rest[2..];
                if rest.len() < token_len {
                    return None;
                }
                let token = String::from_utf8(rest[..token_len].to_vec()).ok()?;
                let consumed = 1 + id_len + 2 + token_len;
                Some((
                    Self::Propose {
                        proposed_id: id,
                        token,
                    },
                    consumed,
                ))
            }
            0x02 => {
                let (id, id_len) = read_varint(rest)?;
                Some((Self::Ack { confirmed_id: id }, 1 + id_len))
            }
            0x03 => {
                let (id, id_len) = read_varint(rest)?;
                Some((Self::Reject { rejected_id: id }, 1 + id_len))
            }
            _ => None,
        }
    }
}

// ── TokenFrequencyTracker ─────────────────────────────────────────────────────

/// Per-connection token frequency tracker for Layer 2 dynamic promotion.
///
/// Observes string tokens as they flow through the connection.  Once a token
/// exceeds `PROMOTION_THRESHOLD` occurrences it is marked as a **promotion
/// candidate** and a `DictSyncMessage::Propose` should be sent to the peer.
///
/// ## ID assignment
///
/// Dynamic IDs are allocated monotonically starting from `DYNAMIC_ID_MIN`.
/// Only tokens that have been ACKed by the peer (via `confirm_promotion`)
/// are inserted into the active decode table.
#[derive(Debug, Default)]
pub struct TokenFrequencyTracker {
    /// Observation counts for unpromotoed tokens.
    frequencies: HashMap<String, u32>,
    /// Promoted and peer-confirmed tokens → dynamic ID.
    active: HashMap<String, u32>,
    /// Reverse map for O(1) decode: dynamic ID → token string.
    reverse: Vec<String>,
    /// Next ID to assign to a newly promoted token.
    next_id: u32,
}

impl TokenFrequencyTracker {
    /// Create a new tracker with an empty frequency table.
    pub fn new() -> Self {
        Self {
            frequencies: HashMap::new(),
            active: HashMap::new(),
            reverse: Vec::new(),
            next_id: DYNAMIC_ID_MIN,
        }
    }

    /// Observe a token occurrence.
    ///
    /// Returns `Some(DictSyncMessage::Propose)` when the token crosses the
    /// promotion threshold and a proposal should be sent to the peer.
    /// Returns `None` for routine observations below the threshold.
    pub fn observe(&mut self, token: &str) -> Option<DictSyncMessage> {
        // Skip if already promoted
        if self.active.contains_key(token) {
            return None;
        }

        let count = self.frequencies.entry(token.to_string()).or_insert(0);
        *count += 1;

        if *count >= PROMOTION_THRESHOLD {
            if self.next_id > DYNAMIC_ID_MAX {
                return None; // dynamic dictionary full
            }
            let proposed_id = self.next_id;
            self.next_id += 1;
            Some(DictSyncMessage::Propose {
                proposed_id,
                token: token.to_string(),
            })
        } else {
            None
        }
    }

    /// Confirm a peer-ACKed promotion, inserting the token into the active table.
    ///
    /// After this call, `encode(token)` will return the dynamic ID and
    /// `decode(id)` will return the token string.
    pub fn confirm_promotion(&mut self, proposed_id: u32, token: &str) {
        let idx = (proposed_id - DYNAMIC_ID_MIN) as usize;
        // Grow reverse table if needed
        if idx >= self.reverse.len() {
            self.reverse.resize(idx + 1, String::new());
        }
        self.reverse[idx] = token.to_string();
        self.active.insert(token.to_string(), proposed_id);
        self.frequencies.remove(token);
    }

    /// Look up the dynamic ID for `token` (encode direction).
    pub fn encode(&self, token: &str) -> Option<u32> {
        self.active.get(token).copied()
    }

    /// Look up the token string for a dynamic `id` (decode direction, zero-alloc).
    pub fn decode(&self, id: u32) -> Option<&str> {
        #[allow(clippy::manual_range_contains)]
        if id < DYNAMIC_ID_MIN || id > DYNAMIC_ID_MAX {
            return None;
        }
        let idx = (id - DYNAMIC_ID_MIN) as usize;
        let s = self.reverse.get(idx)?;
        if s.is_empty() {
            None
        } else {
            Some(s.as_str())
        }
    }

    /// Number of actively promoted tokens.
    pub fn active_count(&self) -> usize {
        self.active.len()
    }
}

// ── Layer 3: LZ4 byte compression ────────────────────────────────────────────

/// Payload size threshold above which Layer 3 LZ4 compression is applied.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "40_Probe_Wire",
        link = "LEP-0014-laplace-probe-wire_protocol_and_compression"
    )
)]
pub const LZ4_COMPRESSION_THRESHOLD: usize = 4 * 1024; // 4 KiB

/// Compress `data` with lz4_flex (size-prepended format).
///
/// Returns the compressed bytes.
pub fn lz4_compress(data: &[u8]) -> Result<Vec<u8>, super::error::MeshError> {
    Ok(lz4_flex::compress_prepend_size(data))
}

/// Decompress LZ4-frame-compressed data.
pub fn lz4_decompress(data: &[u8]) -> Result<Vec<u8>, super::error::MeshError> {
    lz4_flex::decompress_size_prepended(data)
        .map_err(|e| super::error::MeshError::DecompressionError(e.to_string()))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracker_promotes_after_threshold() {
        let mut t = TokenFrequencyTracker::new();
        for _ in 0..PROMOTION_THRESHOLD - 1 {
            assert!(t.observe("GET /api/users").is_none());
        }
        let proposal = t.observe("GET /api/users");
        assert!(matches!(
            proposal,
            Some(DictSyncMessage::Propose {
                proposed_id: 0x8000,
                ..
            })
        ));
    }

    #[test]
    fn tracker_confirm_then_encode_decode() {
        let mut t = TokenFrequencyTracker::new();
        for _ in 0..PROMOTION_THRESHOLD {
            t.observe("POST /orders");
        }
        t.confirm_promotion(DYNAMIC_ID_MIN, "POST /orders");

        assert_eq!(t.encode("POST /orders"), Some(DYNAMIC_ID_MIN));
        assert_eq!(t.decode(DYNAMIC_ID_MIN), Some("POST /orders"));
    }

    #[test]
    fn dict_sync_propose_roundtrip() {
        let demo_token = ["DELETE", "/items"].join(" ");
        let msg = DictSyncMessage::Propose {
            proposed_id: 0x8001,
            token: demo_token,
        };
        let bytes = msg.to_bytes();
        let (decoded, consumed) = DictSyncMessage::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, msg);
        assert_eq!(consumed, bytes.len());
    }

    #[test]
    fn dict_sync_ack_roundtrip() {
        let msg = DictSyncMessage::Ack {
            confirmed_id: 0x8002,
        };
        let bytes = msg.to_bytes();
        let (decoded, _) = DictSyncMessage::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn dict_sync_reject_roundtrip() {
        let msg = DictSyncMessage::Reject {
            rejected_id: 0x8003,
        };
        let bytes = msg.to_bytes();
        let (decoded, _) = DictSyncMessage::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn lz4_compress_decompress_roundtrip() {
        let data: Vec<u8> = (0..5000).map(|i| (i % 256) as u8).collect();
        let compressed = lz4_compress(&data).unwrap();
        let decompressed = lz4_decompress(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }
}
