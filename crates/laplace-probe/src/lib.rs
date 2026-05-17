//! Laplace Probe — QUIC Mesh Sidecar
//!
//! Unified sidecar that combines QUIC transport (formerly `laplace-knul`) with
//! semantic mesh encoding (formerly `laplace-mesh`) into a single deployable unit.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────┐
//! │   Deno TypeScript SDK    │
//! └────────────┬─────────────┘
//!              │ FFI (laplace_probe_init / laplace_probe_start / …)
//!              ▼
//! ┌──────────────────────────┐
//! │  laplace-probe (Rust)    │
//! │  adapters/ffi            │  ← C FFI entry points
//! │  adapters/quinn          │  ← QUIC transport
//! │  infrastructure/         │  ← runtime, registry, queue
//! │  domain/transport        │  ← KnulEndpoint / KnulStream traits
//! │  domain/wire             │  ← SemanticEncoder / SemanticDecoder
//! └──────────────────────────┘
//! ```

pub mod adapters;
pub mod domain;
pub mod infrastructure;

// Re-export public FFI entry points (renamed from krepis_quic_*)
pub use adapters::ffi::laplace_probe_init;
pub use adapters::ffi::{
    laplace_probe_get_stats, laplace_probe_inject_context, laplace_probe_send, laplace_probe_start,
    laplace_probe_stop,
};

// Re-export transport traits for external consumers
pub use domain::transport::{KnulConnection, KnulEndpoint, KnulStream, TransportError};

// Re-export domain event types (formerly in laplace-mesh::client)
pub use domain::events::{EventContext, ProbeEvent};

// Re-export wire codec types for external consumers
pub use domain::wire::{
    read_varint, write_varint, DictSyncMessage, MeshError, SemanticDecoder, SemanticEncoder,
    StaticDictionary, TokenFrequencyTracker, LZ4_COMPRESSION_THRESHOLD, STATIC_ID_MAX,
    STATIC_ID_MIN,
};

// Re-export Phase 3 compression flag constants
pub use adapters::mesh_agent::outbound::{
    make_client_endpoint, FLAG_LAYER1, FLAG_LAYER2, FLAG_LAYER3,
};

// Re-export TLS config helpers for external consumers (e.g. laplace-kraken, laplace-axiom)
pub use adapters::mesh_agent::inbound::make_server_config;

// Re-export core FFI types
pub use laplace_interfaces::{
    FfiBuffer, FfiQuicConfig, FfiResponse, LaplaceError, FFI_ABI_VERSION,
};

/// Library version
pub const LAPLACE_PROBE_VERSION: &str = env!("CARGO_PKG_VERSION");

// Re-export MeshAgent public API
pub use adapters::mesh_agent::{MeshAgent, MeshAgentBuilder, MeshAgentError, MeshAgentRegistry};

// Re-export Phase 2 context types
pub use domain::context::{FfiLaplaceContext, LaplaceContext};
