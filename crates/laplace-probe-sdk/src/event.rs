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
    /// [v2] An async task was spawned.
    TaskSpawned {
        task_id: u64,
        parent_task_id: Option<u64>,
        source_location: Option<String>,
    },
    /// [v2] An async task's root future was polled.
    TaskPolled { task_id: u64, poll_attempt_id: u64 },
    /// [v2] A poll returned Poll::Pending.
    FuturePending {
        task_id: u64,
        future_id: Option<u64>,
        poll_attempt_id: u64,
    },
    /// [v2] A poll returned Poll::Ready.
    FutureReady {
        task_id: u64,
        future_id: Option<u64>,
        poll_attempt_id: u64,
    },
    /// [v2] A waker was invoked (wake causality edge).
    WakeIssued {
        source_task_id: Option<u64>,
        target_task_id: u64,
        waker_id: u64,
    },
    /// [v2] Cancellation was requested for a task.
    CancelRequested { task_id: u64 },
    /// [v2] An async task completed (ready or cancelled).
    TaskCompleted { task_id: u64 },
    /// [v2] A task registered as a waiter on a Notify slot.
    NotifyWaiterRegistered { notify_id: u64, task_id: u64 },
    /// [v2] A notify call stored the single permit bit.
    NotifyStoredPermit { notify_id: u64, bit: bool },
    /// [v2] A notify call woke one registered waiter.
    NotifyWakeEdge {
        notify_id: u64,
        source_task_id: Option<u64>,
        target_task_id: u64,
    },
    /// [v2] A waiter supplied its latest waker identity.
    NotifyLatestWaker {
        notify_id: u64,
        task_id: u64,
        waker_id: u64,
    },
    /// [v2] A notify call was coalesced into an already-stored permit.
    NotifyWakeCoalesced { notify_id: u64 },
    /// [v2] A waiter returned from wait after a wake edge.
    NotifyWaiterCompleted { notify_id: u64, task_id: u64 },
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
            // [v2] Async vocabulary carries task/future/waker identity, not a
            // named resource — the engine has no async mapping yet (AXM2 A2-1+).
            Self::TaskSpawned { .. }
            | Self::TaskPolled { .. }
            | Self::FuturePending { .. }
            | Self::FutureReady { .. }
            | Self::WakeIssued { .. }
            | Self::CancelRequested { .. }
            | Self::TaskCompleted { .. } => None,
            // [v2] Notify vocabulary carries a numeric `notify_id` identity
            // (mirrors `laplace_sync::notify::NotifyEvent`), not a named
            // resource — same policy as the other [v2] async variants above.
            Self::NotifyWaiterRegistered { .. }
            | Self::NotifyStoredPermit { .. }
            | Self::NotifyWakeEdge { .. }
            | Self::NotifyLatestWaker { .. }
            | Self::NotifyWakeCoalesced { .. }
            | Self::NotifyWaiterCompleted { .. } => None,
        }
    }
}
