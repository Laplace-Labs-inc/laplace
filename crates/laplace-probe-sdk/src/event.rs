// SPDX-License-Identifier: Apache-2.0
//! Public probe events emitted by tracked synchronization primitives.

use serde::{Deserialize, Serialize};

/// Runtime event collected by the public instrumentation SDK.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProbeEvent {
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
    /// A shared lock on an RwLock was acquired.
    RwLockReadAcquired { thread_id: u64, resource: String },
    /// A shared lock on an RwLock was released.
    RwLockReadReleased { thread_id: u64, resource: String },
    /// An exclusive lock on an RwLock was acquired.
    RwLockWriteAcquired { thread_id: u64, resource: String },
    /// An exclusive lock on an RwLock was released.
    RwLockWriteReleased { thread_id: u64, resource: String },
    /// An atomic load operation was performed.
    AtomicLoad { thread_id: u64, resource: String },
    /// An atomic store operation was performed.
    AtomicStore { thread_id: u64, resource: String },
    /// An atomic read-modify-write operation was performed.
    AtomicRmw { thread_id: u64, resource: String },
    /// A semaphore was acquired.
    SemaphoreAcquired { thread_id: u64, resource: String },
    /// A semaphore was released.
    SemaphoreReleased { thread_id: u64, resource: String },
}

impl ProbeEvent {
    /// Returns the resource name carried by synchronization events.
    pub fn resource_name(&self) -> Option<&str> {
        match self {
            Self::ThreadBlocked { blocked_on, .. } => Some(blocked_on),
            Self::LockAcquired { resource, .. }
            | Self::LockReleased { resource, .. }
            | Self::RwLockReadAcquired { resource, .. }
            | Self::RwLockReadReleased { resource, .. }
            | Self::RwLockWriteAcquired { resource, .. }
            | Self::RwLockWriteReleased { resource, .. }
            | Self::AtomicLoad { resource, .. }
            | Self::AtomicStore { resource, .. }
            | Self::AtomicRmw { resource, .. }
            | Self::SemaphoreAcquired { resource, .. }
            | Self::SemaphoreReleased { resource, .. } => Some(resource),
            Self::DbQuery { .. } | Self::HttpRequest { .. } | Self::Custom { .. } => None,
        }
    }
}
