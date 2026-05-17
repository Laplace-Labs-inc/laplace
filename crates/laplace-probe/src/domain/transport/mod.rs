// SPDX-License-Identifier: Apache-2.0
// ============================================================================
// FILE: src/domain/transport/mod.rs
// Re-exports for QUIC transport abstraction from laplace-interfaces
// ============================================================================

// Re-export error type to maintain compatibility with existing laplace-knul code
pub use laplace_interfaces::error::TransportError;

// Re-export trait types to maintain compatibility with existing laplace-knul code
pub use laplace_interfaces::domain::transport::{KnulConnection, KnulEndpoint, KnulStream};
