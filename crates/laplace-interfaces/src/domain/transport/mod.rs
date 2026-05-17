//! # Transport Domain Module
//!
//! Authoritative trait contracts and type definitions for all network transport
//! layers in Laplace, including QUIC-based KNUL, abstract SovereignTransport,
//! and simulation-based virtual transports.
//!
//! ## Module Organization
//!
//! - `error`: Transport-specific error types and error code mapping
//! - `knul`: QUIC transport trait contracts (streams, connections, endpoints)
//! - `types`: Data structures and enumerations (packets, stats, HTTP types, seeds)
//! - `traits`: High-level transport abstractions and factory patterns

pub mod error;
pub mod knul;
pub mod pluggable;
pub mod traits;
pub mod types;

// Re-export primary types for consumer convenience
pub use error::TransportError;
pub use knul::{KnulConnection, KnulEndpoint, KnulStream};
pub use pluggable::{
    InterceptReason, NetworkClockProvider, NullInterceptor, OsSocketProvider, PacketBuffer,
    PacketInterceptor, SocketProvider, WallClockProvider,
};
pub use traits::{SovereignTransport, TransportFactory, VirtualTransport};
pub use types::{
    HttpMethod, TransportHandle, TransportPacket, TransportStats, VirtualRequest, VirtualResponse,
    VUID,
};
