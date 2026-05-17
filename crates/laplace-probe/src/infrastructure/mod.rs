//! Infrastructure Layer
//!
//! Low-level system abstractions and runtime management for the Laplace KNUL
//! (Network Utility Link) adapter. Provides server instance management and
//! lifecycle coordination.
//!
//! The infrastructure layer provides:
//! - **registry**: Thread-safe server instance management with handle-based lookups
//! - **queue**: Low-level packet queue for zero-copy handoff
//! - **runtime**: Global runtime and state management
//! - **ffi**: FFI utility functions for type conversion

pub mod chaos;
pub mod ffi;
pub mod queue;
pub mod registry;
pub mod runtime;
pub mod transport;

pub use transport::quinn_impl::{QuinnConnection, QuinnEndpoint, QuinnStream};
