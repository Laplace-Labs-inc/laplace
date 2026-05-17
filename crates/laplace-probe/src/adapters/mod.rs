// SPDX-License-Identifier: Apache-2.0
//! Adapter Layer
//!
//! Protocol-specific implementations of abstract transport and runtime interfaces.
//! Provides concrete adapters for external systems (quinn QUIC, etc.) while
//! maintaining independence from business logic.
//!
//! The adapter layer provides:
//! - **ffi**: FFI entry point handlers
//! - **quinn**: Quinn-based QUIC transport implementation
//!   - handler: Per-connection packet processing
//!   - server: Endpoint lifecycle and statistics
//!   - mod: SovereignTransport trait implementation

pub mod ffi;
pub mod mesh_agent;
pub mod quinn;

pub use mesh_agent::{MeshAgent, MeshAgentBuilder, MeshAgentError, MeshAgentRegistry};
pub use quinn::{QuicServer, QuinnTransport};

pub use ffi::{
    laplace_probe_get_stats, laplace_probe_init, laplace_probe_send, laplace_probe_start,
    laplace_probe_stop,
};
