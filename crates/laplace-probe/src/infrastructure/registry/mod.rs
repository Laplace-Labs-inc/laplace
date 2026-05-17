//! Registry Module
//!
//! Thread-safe server instance registry providing handle-based management
//! of QUIC server instances. Enables safe concurrent access from multiple
//! threads and async tasks through DashMap-backed storage.

mod types;

// Re-export registry types at module level
pub use types::{HandleAllocator, ServerRegistry};
