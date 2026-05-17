// SPDX-License-Identifier: Apache-2.0
//! Domain event types for the Laplace Probe.
//!
//! Defines the events captured by the probe and forwarded to the Axiom Console,
//! as well as control commands from the console.
//!
//! (Migrated from `laplace-mesh::client` as part of the Phase 0 merger.)

use serde::{Deserialize, Serialize};

/// A runtime event captured by the Probe and forwarded to the Axiom Console.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProbeEvent {
    // === 기존 — 변경 금지 ===
    /// A thread was blocked waiting on a resource.
    ThreadBlocked { thread_id: u64, blocked_on: String },
    /// A lock on a shared resource was acquired.
    LockAcquired { thread_id: u64, resource: String },
    /// A lock on a shared resource was released.
    LockReleased { thread_id: u64, resource: String },
    /// A database query was executed.
    DbQuery { query: String, duration_us: u64 },
    /// An HTTP request was handled.
    HttpRequest {
        method: String,
        path: String,
        status_code: u16,
    },
    /// A user-defined custom event with arbitrary metadata.
    Custom {
        name: String,
        metadata: serde_json::Value,
    },

    // === 신규: RwLock ===
    /// A shared (read) lock on an RwLock was acquired.
    RwLockReadAcquired { thread_id: u64, resource: String },
    /// A shared (read) lock on an RwLock was released.
    RwLockReadReleased { thread_id: u64, resource: String },
    /// An exclusive (write) lock on an RwLock was acquired.
    RwLockWriteAcquired { thread_id: u64, resource: String },
    /// An exclusive (write) lock on an RwLock was released.
    RwLockWriteReleased { thread_id: u64, resource: String },

    // === 신규: Atomic ===
    /// An atomic load operation was performed.
    AtomicLoad { thread_id: u64, resource: String },
    /// An atomic store operation was performed.
    AtomicStore { thread_id: u64, resource: String },
    /// An atomic read-modify-write operation was performed (CAS, fetch_add, etc).
    AtomicRmw { thread_id: u64, resource: String },

    // === 신규: Semaphore ===
    /// A semaphore was acquired.
    SemaphoreAcquired { thread_id: u64, resource: String },
    /// A semaphore was released.
    SemaphoreReleased { thread_id: u64, resource: String },
}

/// Contextual envelope wrapping a [`ProbeEvent`] with timing and trace metadata.
#[derive(Debug, Clone)]
pub struct EventContext {
    /// The captured event.
    pub event: ProbeEvent,
    /// Wall-clock timestamp in nanoseconds since the Unix epoch.
    pub timestamp_ns: u64,
    /// Optional distributed trace identifier.
    pub trace_id: Option<String>,
}

impl EventContext {
    /// Creates a new context with the current wall-clock timestamp.
    pub fn new(event: ProbeEvent) -> Self {
        let timestamp_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        Self {
            event,
            timestamp_ns,
            trace_id: None,
        }
    }

    /// Attaches a distributed trace identifier to this context.
    pub fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }
}
