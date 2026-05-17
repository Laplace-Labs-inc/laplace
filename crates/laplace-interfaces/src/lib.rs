//! # Laplace Interfaces
//!
//! Pure interface definitions for Laplace (FFI, Types, Traits).
//!
//! This crate is the authoritative source of truth for all types and traits
//! shared across the Laplace ecosystem. Domain models, FFI contracts, and
//! business logic enumerations are defined here to ensure consistency and
//! enable deterministic code generation to TypeScript.
//!
//! ## Design Principles
//!
//! - **Single Source of Truth**: All type definitions are centralized here.
//!   Consumers in `laplace-core`, `laplace-axiom`, `laplace-kraken`, etc. depend
//!   on this crate, never on each other's type definitions.
//!
//! - **Zero Coupling**: Types in this crate have no dependencies on domain logic.
//!   They define contracts only; implementations live in consumers.
//!
//!
//! ## Module Organization
//!
//! - `abi`: FFI layer contracts (Sovereign Bridge ABI v1.1.0)
//! - `error`: Error types and domain-specific error handling
//! - `domain`: Domain models and shared business logic types

#![warn(missing_docs)]
#![allow(non_snake_case)]

pub mod abi;
pub mod domain;
pub mod error;

// Re-export common types at crate level for convenience
pub use abi::{
    AxiomConfig, ConfigSyncError, ConfigSynchronizer, FfiBuffer, FfiLockState, FfiQuicConfig,
    FfiResponse, FfiValidatable, KrakenConfig, LaplaceConfig, LaplaceGlobalConfig, ProbeConfig,
    SharedMemoryMetadata, FFI_ABI_VERSION, FFI_BUFFER_ALIGN,
};

pub use error::{LaplaceError, TenantError};

pub use domain::{
    HttpMethod, KernelCapabilities, KnulConnection, KnulEndpoint, KnulStream, PanelType,
    PriorityLevel, QuicServerStats, ResourceConfig, RuntimeStats, SovereignContext,
    SovereignRuntime, SovereignTransport, TenantMetadata, TenantTier, Tier, TransportError,
    TransportFactory, TransportHandle, TransportPacket, TransportStats, TuiCapabilities,
    VirtualRequest, VirtualResponse, VirtualTransport, NO_TURBO_SLOT, VUID,
};

#[cfg(test)]
mod lib_tests {
    use super::*;

    #[test]
    fn version_constants_defined() {
        assert_eq!(FFI_ABI_VERSION, 0x00010001);
    }

    #[test]
    fn abi_types_accessible() {
        // Verify that all ABI types are re-exported at the crate root
        let _buffer = FfiBuffer::new();
        let _response = FfiResponse::success(FfiBuffer::new());
        let _config = FfiQuicConfig::new();
        let _metadata = SharedMemoryMetadata::new(1, 32, 1024);
    }

    #[test]
    fn error_types_accessible() {
        let _error = LaplaceError::Internal;
    }

    #[test]
    fn domain_runtime_accessible() {
        // Verify that runtime types are re-exported at crate root
        let stats = RuntimeStats::new();
        assert_eq!(stats.isolate_count, 0);
        assert_eq!(stats.avg_exec_us(), 0.0);
    }

    #[test]
    fn domain_transport_accessible() {
        // Verify that transport types are re-exported at crate root
        let packet = TransportPacket::new(vec![1, 2, 3], 42);
        assert_eq!(packet.connection_id, 42);
        assert_eq!(packet.len(), 3);

        let stats = TransportStats::default();
        assert_eq!(stats.total_packets_received, 0);
    }
}
