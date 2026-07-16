// SPDX-License-Identifier: Apache-2.0
//! Public probe events emitted by tracked synchronization primitives.

use serde::{Deserialize, Serialize};

/// 런타임 async lock 획득 어휘의 probe 측 미러다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AsyncAcquireKind {
    /// Mutex 획득.
    Mutex,
    /// 공유 async RwLock 획득.
    RwRead,
    /// 배타 async RwLock 획득.
    RwWrite,
    /// 지정한 permit 수의 semaphore 획득.
    SemaphorePermits(u32),
}

/// 런타임 async channel 종류 어휘의 probe 측 미러다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AsyncChannelKind {
    /// 설정된 capacity를 가진 bounded mpsc channel.
    MpscBounded { capacity: usize },
    /// Unbounded mpsc channel.
    MpscUnbounded,
    /// Oneshot channel.
    Oneshot,
    /// Watch channel.
    Watch,
}

/// 런타임 async channel endpoint 어휘의 probe 측 미러다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AsyncChannelSide {
    /// 송신 endpoint.
    Sender,
    /// 수신 endpoint.
    Receiver,
}

/// 런타임 async channel operation 어휘의 probe 측 미러다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AsyncChannelOp {
    /// 송신 operation.
    Send,
    /// 수신 operation.
    Recv,
    /// Watch 변경 operation.
    Changed,
}

/// 런타임 async channel 결과 어휘의 probe 측 미러다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AsyncChannelOutcome {
    /// Operation이 성공적으로 완료됨.
    Ok,
    /// Channel이 닫힘.
    Closed,
    /// Non-blocking receive에서 값이 없음.
    Empty,
    /// Non-blocking send에서 capacity가 없음.
    Full,
}

/// W broadcast operation vocabulary mirrored from `laplace_rt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BroadcastOp {
    Send,
    Recv,
    TryRecv,
    Resubscribe,
}

/// W broadcast outcome vocabulary mirrored from `laplace_rt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BroadcastOutcome {
    Ok { receivers: usize },
    Closed,
    Empty,
    Lagged { missed: u64 },
}

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
    /// [v2] Async lock family 획득이 요청됨.
    AsyncLockRequested {
        thread_id: u64,
        resource: u64,
        waiter: u64,
        kind: AsyncAcquireKind,
    },
    /// [v2] Async lock family 획득이 해결됨.
    AsyncLockAcquired {
        thread_id: u64,
        resource: u64,
        waiter: u64,
        kind: AsyncAcquireKind,
    },
    /// [v2] Async lock family guard 또는 permit이 해제됨.
    AsyncLockReleased {
        thread_id: u64,
        resource: u64,
        waiter: u64,
        kind: AsyncAcquireKind,
    },
    /// [v2] Async lock family waiter가 해결 전에 drop됨.
    AsyncLockWaiterDropped {
        thread_id: u64,
        resource: u64,
        waiter: u64,
    },
    /// [v2] Async semaphore resource가 처음 관측됨.
    AsyncSemaphoreCreated {
        thread_id: u64,
        resource: u64,
        permits: usize,
    },
    /// [v2] Async semaphore capacity가 증가됨.
    AsyncPermitsAdded {
        thread_id: u64,
        resource: u64,
        n: usize,
    },
    /// [v2] Async Notify waiter가 요청됨.
    AsyncNotifyWaitRequested {
        thread_id: u64,
        resource: u64,
        waiter: u64,
    },
    /// [v2] Async Notify waiter가 해결됨.
    AsyncNotifyWaitResolved {
        thread_id: u64,
        resource: u64,
        waiter: u64,
    },
    /// [v2] Async Notify `notify_one` 경계가 발생함.
    AsyncNotifyOne { thread_id: u64, resource: u64 },
    /// [v2] Async Notify `notify_waiters` 경계가 발생함.
    AsyncNotifyWaiters { thread_id: u64, resource: u64 },
    /// [v2] Async Notify waiter가 해결 전에 drop됨.
    AsyncNotifyWaiterDropped {
        thread_id: u64,
        resource: u64,
        waiter: u64,
    },
    /// [v2] Async channel이 생성됨.
    AsyncChannelCreated {
        thread_id: u64,
        channel: u64,
        kind: AsyncChannelKind,
    },
    /// [v2] Async channel operation이 요청됨.
    AsyncChannelOpRequested {
        thread_id: u64,
        channel: u64,
        op: u64,
        op_kind: AsyncChannelOp,
    },
    /// [v2] Async channel operation이 결과와 함께 해결됨.
    AsyncChannelOpResolved {
        thread_id: u64,
        channel: u64,
        op: u64,
        op_kind: AsyncChannelOp,
        outcome: AsyncChannelOutcome,
    },
    /// [v2] Async channel operation이 해결 전에 drop됨.
    AsyncChannelOpDropped {
        thread_id: u64,
        channel: u64,
        op: u64,
    },
    /// [v2] Async channel endpoint가 clone됨.
    AsyncChannelEndpointCloned {
        thread_id: u64,
        channel: u64,
        side: AsyncChannelSide,
    },
    /// [v2] Async channel endpoint가 drop됨.
    AsyncChannelEndpointDropped {
        thread_id: u64,
        channel: u64,
        side: AsyncChannelSide,
    },
    /// [v2] Async channel receiver가 닫힘.
    AsyncChannelClosed { thread_id: u64, channel: u64 },
    /// W broadcast resource가 생성됨. The engine still treats this surface
    /// as unmodeled capture data.
    AsyncBroadcastCreated {
        thread_id: u64,
        resource: u64,
        capacity: usize,
    },
    /// W broadcast receiver가 구독함.
    AsyncBroadcastSubscribed {
        thread_id: u64,
        resource: u64,
        receiver_id: u64,
        at_seq: u64,
    },
    /// W broadcast operation이 요청됨.
    AsyncBroadcastOpRequested {
        thread_id: u64,
        resource: u64,
        op: u64,
        receiver_id: Option<u64>,
        op_kind: BroadcastOp,
    },
    /// W broadcast operation이 해결됨.
    AsyncBroadcastOpResolved {
        thread_id: u64,
        resource: u64,
        op: u64,
        receiver_id: Option<u64>,
        op_kind: BroadcastOp,
        outcome: BroadcastOutcome,
    },
    /// W broadcast operation이 해결 전에 drop됨.
    AsyncBroadcastOpDropped {
        thread_id: u64,
        resource: u64,
        op: u64,
    },
    /// W broadcast endpoint가 clone됨.
    AsyncBroadcastEndpointCloned {
        thread_id: u64,
        resource: u64,
        side: AsyncChannelSide,
        receiver_id: Option<u64>,
    },
    /// W broadcast endpoint가 drop됨.
    AsyncBroadcastEndpointDropped {
        thread_id: u64,
        resource: u64,
        side: AsyncChannelSide,
        receiver_id: Option<u64>,
    },
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
            | Self::NotifyWaiterCompleted { .. }
            // [v2] 런타임 async hook은 숫자 identity를 가지며 engine 측
            // Notify 어휘와 의도적으로 분리한다.
            | Self::AsyncLockRequested { .. }
            | Self::AsyncLockAcquired { .. }
            | Self::AsyncLockReleased { .. }
            | Self::AsyncLockWaiterDropped { .. }
            | Self::AsyncSemaphoreCreated { .. }
            | Self::AsyncPermitsAdded { .. }
            | Self::AsyncNotifyWaitRequested { .. }
            | Self::AsyncNotifyWaitResolved { .. }
            | Self::AsyncNotifyOne { .. }
            | Self::AsyncNotifyWaiters { .. }
            | Self::AsyncNotifyWaiterDropped { .. }
            | Self::AsyncChannelCreated { .. }
            | Self::AsyncChannelOpRequested { .. }
            | Self::AsyncChannelOpResolved { .. }
            | Self::AsyncChannelOpDropped { .. }
            | Self::AsyncChannelEndpointCloned { .. }
            | Self::AsyncChannelEndpointDropped { .. }
            | Self::AsyncChannelClosed { .. }
            | Self::AsyncBroadcastCreated { .. }
            | Self::AsyncBroadcastSubscribed { .. }
            | Self::AsyncBroadcastOpRequested { .. }
            | Self::AsyncBroadcastOpResolved { .. }
            | Self::AsyncBroadcastOpDropped { .. }
            | Self::AsyncBroadcastEndpointCloned { .. }
            | Self::AsyncBroadcastEndpointDropped { .. } => None,
        }
    }
}
