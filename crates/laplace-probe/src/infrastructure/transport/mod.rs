// ============================================================================
// FILE: src/infrastructure/transport/mod.rs
// Infrastructure module re-exports
// ============================================================================

pub mod quinn_impl;
pub mod virtual_socket;

// Re-export for convenience, but users should use trait objects
pub use quinn_impl::{QuinnConnection, QuinnEndpoint, QuinnStream};
