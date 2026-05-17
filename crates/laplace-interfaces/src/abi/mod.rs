//! # Sovereign Bridge ABI v1.1.0
//!
//! Low-level FFI memory layout definitions for Rust-Deno zero-copy bridge.
//! All structures use `#[repr(C)]` with explicit alignment for ABI stability.
//!
//! ## Design Principles
//!
//! The Sovereign Bridge ABI establishes a stable contract between the Laplace
//! kernel (Rust) and the TypeScript SDK (Deno) running in separate processes.
//! This ABI defines:
//!
//! - Explicit memory layouts with compile-time verification
//! - Pointer safety guarantees across process boundaries
//! - Thread-safe data transfer mechanisms
//! - Version compatibility checking
//!
//! ## Module Organization
//!
//! - `primitives`: Basic FFI types (FfiBuffer, FfiResponse, FfiLockState)
//! - `config`: QUIC server configuration structures
//! - `shared`: Shared memory coordination metadata

pub mod config;
pub mod primitives;
pub mod shared;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

// Re-export all ABI types at module level for consumer convenience
pub use config::{
    AxiomConfig, ConfigSyncError, ConfigSynchronizer, FfiQuicConfig, KrakenConfig, LaplaceConfig,
    LaplaceGlobalConfig, NetworkConfig, ProbeConfig, ResourceLimitConfig, VerificationConfig,
};
pub use primitives::{FfiBuffer, FfiLockState, FfiResponse};
pub use shared::SharedMemoryMetadata;

// ============================================================================
// ABI Constants
// ============================================================================

/// FFI ABI Version
///
/// Version encoding: `0xMMmmRRRR` where MM = major, mm = minor, RRRR = revision
/// Current: 1.1.0 (Major 1, Minor 1, Revision 0)
///
/// Used for runtime compatibility checking at FFI boundary.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_ABI",
        link = "LEP-0003-laplace-interfaces-shared_memory_abi"
    )
)]
pub const FFI_ABI_VERSION: u32 = 0x00010001;

/// Standard alignment for all FFI structures
///
/// All FFI types must be 8-byte aligned to satisfy:
/// - 64-bit architecture requirements
/// - Atomic instruction boundaries
/// - Cache-line alignment for lock-free algorithms
pub const FFI_BUFFER_ALIGN: usize = 8;

// ============================================================================
// FFI Validation Contract
// ============================================================================

/// Core validation trait for all FFI-safe types.
///
/// Every type that crosses the Rust-Deno boundary must validate its invariants
/// before use. This trait enforces the contract that all FFI structures are
/// responsible for self-validation.
///
/// # Implementation Requirements
///
/// Implementing types must verify:
/// - All pointer fields are either valid or null (never uninitialized)
/// - Numeric constraints (non-zero ports, positive timeouts, etc.)
/// - Buffer lengths do not exceed capacities
/// - Dependent fields maintain consistency
///
/// # Example
///
/// ```ignore
/// impl FfiValidatable for FfiQuicConfig {
///     fn is_valid(&self) -> bool {
///         // All constraints checked, none fail → true
///         self.port != 0 && self.max_streams != 0 && /* ... */
///     }
/// }
/// ```
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_ABI",
        link = "LEP-0003-laplace-interfaces-shared_memory_abi"
    )
)]
pub trait FfiValidatable {
    /// Validate all invariants for this FFI type.
    ///
    /// Returns `true` if all constraints are satisfied, `false` otherwise.
    /// No panics; failures are communicated via return value.
    fn is_valid(&self) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_version_encoding() {
        // Verify version encoding: 0x00010001 = 1.1.0
        let major = (FFI_ABI_VERSION >> 24) & 0xFF;
        let minor = (FFI_ABI_VERSION >> 16) & 0xFF;

        assert_eq!(major, 0, "Major version should be 0 (corresponds to 1.x)");
        assert_eq!(minor, 1, "Minor version should be 1 (1.1.x)");
    }

    #[test]
    fn ffi_buffer_align_constant() {
        assert_eq!(FFI_BUFFER_ALIGN, 8);
        assert!(FFI_BUFFER_ALIGN.is_power_of_two());
    }
}
