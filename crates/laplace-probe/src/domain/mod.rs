//! Domain Layer
//!
//! Core domain data structures for KNUL network operations.
//! Defines types for packet handling, statistics, and trait conversions.

pub mod context;
pub mod events;
pub mod transport;
pub mod types;
pub mod wire;

// Re-export domain types at module level
pub use transport::{KnulConnection, KnulEndpoint, KnulStream, TransportError};
pub use types::PacketBuffer;
