// SPDX-License-Identifier: Apache-2.0
//! Deterministic context injected into QUIC stream frames.
//!
//! `LaplaceContext` carries the tracing, tenancy, and virtual-clock state
//! that the Axiom engine needs to deterministically replay distributed AI
//! agent communication.
//!
//! ## Wire encoding (41 bytes, little-endian)
//!
//! ```text
//! Offset  Size  Field
//! 0       16    trace_id (u128 LE)
//! 16       8    tenant_id (u64 LE)
//! 24       8    virtual_clock_ns (u64 LE)
//! 32       8    lamport_tick (u64 LE)
//! 40       1    priority (u8)
//! ```
//!
//! ## Frame format (used by MeshAgent batch flusher / inbound reader)
//!
//! ```text
//! [4 bytes BE: total_frame_len]          ← length of everything after this field
//! [1 byte: flags]                        ← bit 4 (0x10) set ↔ context present
//! [41 bytes: LaplaceContext encoding]    ← only when flags & CTX_FLAG != 0
//! [N bytes: payload]
//! ```

/// Flag bit indicating that a `LaplaceContext` header is present in the frame.
pub const CTX_FLAG: u8 = 0x10;

/// Size of a serialised [`LaplaceContext`] in bytes.
pub const CONTEXT_BYTES: usize = 41;

// ── LaplaceContext ────────────────────────────────────────────────────────────

/// Deterministic context for distributed trace, tenant isolation, and
/// clock synchronisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LaplaceContext {
    /// 128-bit distributed trace identifier (UUID / Snowflake compatible).
    pub trace_id: u128,
    /// Tenant / isolate identifier for multi-tenant routing.
    pub tenant_id: u64,
    /// Logical time from the injected `NetworkClockProvider` (nanoseconds).
    /// Uses `WallClockProvider` in production; `VirtualClock` in Axiom replay.
    pub virtual_clock_ns: u64,
    /// Lamport logical clock — monotonically incremented on every send.
    pub lamport_tick: u64,
    /// Priority hint (0 = lowest, 255 = highest).
    pub priority: u8,
}

impl LaplaceContext {
    /// Serialised length in bytes (`CONTEXT_BYTES`).
    pub const SERIALIZED_LEN: usize = CONTEXT_BYTES;

    /// Serialise to a fixed-size byte array (little-endian).
    pub fn to_bytes(&self) -> [u8; CONTEXT_BYTES] {
        let mut buf = [0u8; CONTEXT_BYTES];
        buf[0..16].copy_from_slice(&self.trace_id.to_le_bytes());
        buf[16..24].copy_from_slice(&self.tenant_id.to_le_bytes());
        buf[24..32].copy_from_slice(&self.virtual_clock_ns.to_le_bytes());
        buf[32..40].copy_from_slice(&self.lamport_tick.to_le_bytes());
        buf[40] = self.priority;
        buf
    }

    /// Deserialise from a fixed-size byte array (little-endian).
    pub fn from_bytes(buf: &[u8; CONTEXT_BYTES]) -> Self {
        Self {
            trace_id: u128::from_le_bytes(buf[0..16].try_into().unwrap()),
            tenant_id: u64::from_le_bytes(buf[16..24].try_into().unwrap()),
            virtual_clock_ns: u64::from_le_bytes(buf[24..32].try_into().unwrap()),
            lamport_tick: u64::from_le_bytes(buf[32..40].try_into().unwrap()),
            priority: buf[40],
        }
    }
}

// ── FfiLaplaceContext ─────────────────────────────────────────────────────────

/// C-ABI–compatible representation of [`LaplaceContext`].
///
/// Foreign AI agents (Python/TypeScript/Deno) allocate this struct on their
/// side and pass a pointer to [`laplace_probe_inject_context`].
///
/// Memory layout: 48 bytes, all fields aligned, `repr(C)`.
///
/// [`laplace_probe_inject_context`]: crate::adapters::ffi::context
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FfiLaplaceContext {
    /// Lower 64 bits of the 128-bit trace ID.
    pub trace_id_lo: u64,
    /// Upper 64 bits of the 128-bit trace ID.
    pub trace_id_hi: u64,
    pub tenant_id: u64,
    pub virtual_clock_ns: u64,
    pub lamport_tick: u64,
    pub priority: u8,
    /// Explicit padding to 8-byte boundary (must be zeroed by the caller).
    pub _padding: [u8; 7],
}

// Size assertion: 48 bytes
const _: () = assert!(
    core::mem::size_of::<FfiLaplaceContext>() == 48,
    "FfiLaplaceContext must be exactly 48 bytes"
);

impl From<FfiLaplaceContext> for LaplaceContext {
    fn from(f: FfiLaplaceContext) -> Self {
        let trace_id = (f.trace_id_hi as u128) << 64 | f.trace_id_lo as u128;
        Self {
            trace_id,
            tenant_id: f.tenant_id,
            virtual_clock_ns: f.virtual_clock_ns,
            lamport_tick: f.lamport_tick,
            priority: f.priority,
        }
    }
}

impl From<LaplaceContext> for FfiLaplaceContext {
    fn from(c: LaplaceContext) -> Self {
        Self {
            trace_id_lo: c.trace_id as u64,
            trace_id_hi: (c.trace_id >> 64) as u64,
            tenant_id: c.tenant_id,
            virtual_clock_ns: c.virtual_clock_ns,
            lamport_tick: c.lamport_tick,
            priority: c.priority,
            _padding: [0u8; 7],
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_bytes() {
        let ctx = LaplaceContext {
            trace_id: 0xDEAD_BEEF_CAFE_1234_ABCD_EF01_2345_6789,
            tenant_id: 42,
            virtual_clock_ns: 1_000_000_007,
            lamport_tick: 99,
            priority: 3,
        };
        let bytes = ctx.to_bytes();
        let decoded = LaplaceContext::from_bytes(&bytes);
        assert_eq!(ctx, decoded);
    }

    #[test]
    fn ffi_roundtrip() {
        let ctx = LaplaceContext {
            trace_id: 0xAABB_CCDD_EEFF_0011_2233_4455_6677_8899,
            tenant_id: 7,
            virtual_clock_ns: 500_000,
            lamport_tick: 1,
            priority: 255,
        };
        let ffi: FfiLaplaceContext = ctx.into();
        let back: LaplaceContext = ffi.into();
        assert_eq!(ctx, back);
    }

    #[test]
    fn ctx_flag_value() {
        assert_eq!(CTX_FLAG, 0x10);
    }
}
