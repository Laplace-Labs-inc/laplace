// SPDX-License-Identifier: Apache-2.0
//! 런타임 async lock, notify, channel seam을 위한 probe hook이다.
//!
//! Timer hook은 의도적으로 여기서 설치하지 않는다. `AsyncTimerHook`에는
//! 시간 권위인 `now_nanos()`가 있으므로 probe가 그 값을 답하면 native 실행
//! 시간이 가상화된다. 따라서 native capture는 runtime의 wrap-real 경로에
//! timer 동작을 맡기고, 이 사각지대를 후속 경계 작업에서 문서화한다.

use std::sync::Arc;

use crate::event::{
    AsyncAcquireKind, AsyncChannelKind, AsyncChannelOp, AsyncChannelOutcome, AsyncChannelSide,
    BroadcastOp, BroadcastOutcome,
};
use crate::session::{current_thread_id, emit};
use crate::ProbeEvent;

impl From<laplace_rt::AsyncAcquireKind> for AsyncAcquireKind {
    fn from(kind: laplace_rt::AsyncAcquireKind) -> Self {
        match kind {
            laplace_rt::AsyncAcquireKind::Mutex => Self::Mutex,
            laplace_rt::AsyncAcquireKind::RwRead => Self::RwRead,
            laplace_rt::AsyncAcquireKind::RwWrite => Self::RwWrite,
            laplace_rt::AsyncAcquireKind::SemaphorePermits(permits) => {
                Self::SemaphorePermits(permits)
            }
        }
    }
}

impl From<laplace_rt::AsyncChannelKind> for AsyncChannelKind {
    fn from(kind: laplace_rt::AsyncChannelKind) -> Self {
        match kind {
            laplace_rt::AsyncChannelKind::MpscBounded { capacity } => {
                Self::MpscBounded { capacity }
            }
            laplace_rt::AsyncChannelKind::MpscUnbounded => Self::MpscUnbounded,
            laplace_rt::AsyncChannelKind::Oneshot => Self::Oneshot,
            laplace_rt::AsyncChannelKind::Watch => Self::Watch,
            _ => unreachable!("laplace-probe-sdk channel kind mirror out of date"),
        }
    }
}

impl From<laplace_rt::AsyncChannelSide> for AsyncChannelSide {
    fn from(side: laplace_rt::AsyncChannelSide) -> Self {
        match side {
            laplace_rt::AsyncChannelSide::Sender => Self::Sender,
            laplace_rt::AsyncChannelSide::Receiver => Self::Receiver,
            _ => unreachable!("laplace-probe-sdk channel side mirror out of date"),
        }
    }
}

impl From<laplace_rt::AsyncChannelOp> for AsyncChannelOp {
    fn from(op: laplace_rt::AsyncChannelOp) -> Self {
        match op {
            laplace_rt::AsyncChannelOp::Send => Self::Send,
            laplace_rt::AsyncChannelOp::Recv => Self::Recv,
            laplace_rt::AsyncChannelOp::Changed => Self::Changed,
            _ => unreachable!("laplace-probe-sdk channel op mirror out of date"),
        }
    }
}

impl From<laplace_rt::AsyncChannelOutcome> for AsyncChannelOutcome {
    fn from(outcome: laplace_rt::AsyncChannelOutcome) -> Self {
        match outcome {
            laplace_rt::AsyncChannelOutcome::Ok => Self::Ok,
            laplace_rt::AsyncChannelOutcome::Closed => Self::Closed,
            laplace_rt::AsyncChannelOutcome::Empty => Self::Empty,
            laplace_rt::AsyncChannelOutcome::Full => Self::Full,
            _ => unreachable!("laplace-probe-sdk channel outcome mirror out of date"),
        }
    }
}

impl From<laplace_rt::AsyncBroadcastOp> for BroadcastOp {
    fn from(op: laplace_rt::AsyncBroadcastOp) -> Self {
        match op {
            laplace_rt::AsyncBroadcastOp::Send => Self::Send,
            laplace_rt::AsyncBroadcastOp::Recv => Self::Recv,
            laplace_rt::AsyncBroadcastOp::TryRecv => Self::TryRecv,
            laplace_rt::AsyncBroadcastOp::Resubscribe => Self::Resubscribe,
            _ => unreachable!("laplace-probe-sdk broadcast op mirror out of date"),
        }
    }
}

impl From<laplace_rt::AsyncBroadcastOutcome> for BroadcastOutcome {
    fn from(outcome: laplace_rt::AsyncBroadcastOutcome) -> Self {
        match outcome {
            laplace_rt::AsyncBroadcastOutcome::Ok { receivers } => Self::Ok { receivers },
            laplace_rt::AsyncBroadcastOutcome::Closed => Self::Closed,
            laplace_rt::AsyncBroadcastOutcome::Empty => Self::Empty,
            laplace_rt::AsyncBroadcastOutcome::Lagged { missed } => Self::Lagged { missed },
            _ => unreachable!("laplace-probe-sdk broadcast outcome mirror out of date"),
        }
    }
}

/// 런타임 async lock family hook의 probe 투영.
pub struct ProbeAsyncLockHook;

impl laplace_rt::AsyncLockHook for ProbeAsyncLockHook {
    fn requested(&self, resource: u64, waiter: u64, kind: laplace_rt::AsyncAcquireKind) {
        emit(ProbeEvent::AsyncLockRequested {
            thread_id: current_thread_id(),
            resource,
            waiter,
            kind: kind.into(),
        });
    }

    fn acquired(&self, resource: u64, waiter: u64, kind: laplace_rt::AsyncAcquireKind) {
        emit(ProbeEvent::AsyncLockAcquired {
            thread_id: current_thread_id(),
            resource,
            waiter,
            kind: kind.into(),
        });
    }

    fn released(&self, resource: u64, waiter: u64, kind: laplace_rt::AsyncAcquireKind) {
        emit(ProbeEvent::AsyncLockReleased {
            thread_id: current_thread_id(),
            resource,
            waiter,
            kind: kind.into(),
        });
    }

    fn waiter_dropped(&self, resource: u64, waiter: u64) {
        emit(ProbeEvent::AsyncLockWaiterDropped {
            thread_id: current_thread_id(),
            resource,
            waiter,
        });
    }

    fn semaphore_created(&self, resource: u64, permits: usize) {
        emit(ProbeEvent::AsyncSemaphoreCreated {
            thread_id: current_thread_id(),
            resource,
            permits,
        });
    }

    fn permits_added(&self, resource: u64, n: usize) {
        emit(ProbeEvent::AsyncPermitsAdded {
            thread_id: current_thread_id(),
            resource,
            n,
        });
    }
}

/// 런타임 async Notify hook의 probe 투영.
pub struct ProbeAsyncNotifyHook;

impl laplace_rt::AsyncNotifyHook for ProbeAsyncNotifyHook {
    fn wait_requested(&self, resource: u64, waiter: u64) {
        emit(ProbeEvent::AsyncNotifyWaitRequested {
            thread_id: current_thread_id(),
            resource,
            waiter,
        });
    }

    fn wait_resolved(&self, resource: u64, waiter: u64) {
        emit(ProbeEvent::AsyncNotifyWaitResolved {
            thread_id: current_thread_id(),
            resource,
            waiter,
        });
    }

    fn notify_one(&self, resource: u64) {
        emit(ProbeEvent::AsyncNotifyOne {
            thread_id: current_thread_id(),
            resource,
        });
    }

    fn notify_waiters(&self, resource: u64) {
        emit(ProbeEvent::AsyncNotifyWaiters {
            thread_id: current_thread_id(),
            resource,
        });
    }

    fn waiter_dropped(&self, resource: u64, waiter: u64) {
        emit(ProbeEvent::AsyncNotifyWaiterDropped {
            thread_id: current_thread_id(),
            resource,
            waiter,
        });
    }
}

/// 런타임 async channel hook의 probe 투영.
pub struct ProbeAsyncChannelHook;

impl laplace_rt::AsyncChannelHook for ProbeAsyncChannelHook {
    fn channel_created(&self, channel: u64, kind: laplace_rt::AsyncChannelKind) {
        emit(ProbeEvent::AsyncChannelCreated {
            thread_id: current_thread_id(),
            channel,
            kind: kind.into(),
        });
    }

    fn op_requested(&self, channel: u64, op: u64, kind: laplace_rt::AsyncChannelOp) {
        emit(ProbeEvent::AsyncChannelOpRequested {
            thread_id: current_thread_id(),
            channel,
            op,
            op_kind: kind.into(),
        });
    }

    fn op_resolved(
        &self,
        channel: u64,
        op: u64,
        kind: laplace_rt::AsyncChannelOp,
        outcome: laplace_rt::AsyncChannelOutcome,
    ) {
        emit(ProbeEvent::AsyncChannelOpResolved {
            thread_id: current_thread_id(),
            channel,
            op,
            op_kind: kind.into(),
            outcome: outcome.into(),
        });
    }

    fn op_dropped(&self, channel: u64, op: u64) {
        emit(ProbeEvent::AsyncChannelOpDropped {
            thread_id: current_thread_id(),
            channel,
            op,
        });
    }

    fn endpoint_cloned(&self, channel: u64, side: laplace_rt::AsyncChannelSide) {
        emit(ProbeEvent::AsyncChannelEndpointCloned {
            thread_id: current_thread_id(),
            channel,
            side: side.into(),
        });
    }

    fn endpoint_dropped(&self, channel: u64, side: laplace_rt::AsyncChannelSide) {
        emit(ProbeEvent::AsyncChannelEndpointDropped {
            thread_id: current_thread_id(),
            channel,
            side: side.into(),
        });
    }

    fn channel_closed(&self, channel: u64) {
        emit(ProbeEvent::AsyncChannelClosed {
            thread_id: current_thread_id(),
            channel,
        });
    }
}

/// W broadcast hook의 probe 투영. 이벤트는 capture 봉투에만 실리고
/// 현재 엔진 판정 경로에는 소비되지 않는다.
pub struct ProbeAsyncBroadcastHook;

impl laplace_rt::AsyncBroadcastHook for ProbeAsyncBroadcastHook {
    fn broadcast_created(&self, resource: u64, capacity: usize) {
        emit(ProbeEvent::AsyncBroadcastCreated {
            thread_id: current_thread_id(),
            resource,
            capacity,
        });
    }

    fn subscribed(&self, resource: u64, receiver_id: u64, at_seq: u64) {
        emit(ProbeEvent::AsyncBroadcastSubscribed {
            thread_id: current_thread_id(),
            resource,
            receiver_id,
            at_seq,
        });
    }

    fn op_requested(
        &self,
        resource: u64,
        op: u64,
        receiver_id: Option<u64>,
        kind: laplace_rt::AsyncBroadcastOp,
    ) {
        emit(ProbeEvent::AsyncBroadcastOpRequested {
            thread_id: current_thread_id(),
            resource,
            op,
            receiver_id,
            op_kind: kind.into(),
        });
    }

    fn op_resolved(
        &self,
        resource: u64,
        op: u64,
        receiver_id: Option<u64>,
        kind: laplace_rt::AsyncBroadcastOp,
        outcome: laplace_rt::AsyncBroadcastOutcome,
    ) {
        emit(ProbeEvent::AsyncBroadcastOpResolved {
            thread_id: current_thread_id(),
            resource,
            op,
            receiver_id,
            op_kind: kind.into(),
            outcome: outcome.into(),
        });
    }

    fn op_dropped(&self, resource: u64, op: u64) {
        emit(ProbeEvent::AsyncBroadcastOpDropped {
            thread_id: current_thread_id(),
            resource,
            op,
        });
    }

    fn endpoint_cloned(
        &self,
        resource: u64,
        side: laplace_rt::AsyncChannelSide,
        receiver_id: Option<u64>,
    ) {
        emit(ProbeEvent::AsyncBroadcastEndpointCloned {
            thread_id: current_thread_id(),
            resource,
            side: side.into(),
            receiver_id,
        });
    }

    fn endpoint_dropped(
        &self,
        resource: u64,
        side: laplace_rt::AsyncChannelSide,
        receiver_id: Option<u64>,
    ) {
        emit(ProbeEvent::AsyncBroadcastEndpointDropped {
            thread_id: current_thread_id(),
            resource,
            side: side.into(),
            receiver_id,
        });
    }
}

/// ArcSwap evidence cell hook의 probe 투영.
pub struct ProbeAsyncCellHook;

impl laplace_rt::AsyncCellHook for ProbeAsyncCellHook {
    fn cell_created(&self, resource: u64) {
        emit(ProbeEvent::AsyncCellCreated {
            thread_id: current_thread_id(),
            resource,
        });
    }

    fn cell_load(&self, resource: u64, version: u64) {
        emit(ProbeEvent::AsyncCellLoad {
            thread_id: current_thread_id(),
            resource,
            version,
        });
    }

    fn cell_store(&self, resource: u64, version: u64) {
        emit(ProbeEvent::AsyncCellStore {
            thread_id: current_thread_id(),
            resource,
            version,
        });
    }
}

/// Timer hook을 제외한 모든 probe async hook을 설치한다.
pub fn install_probe_async_hooks() {
    laplace_rt::install_async_lock_hook(Arc::new(ProbeAsyncLockHook));
    laplace_rt::install_async_notify_hook(Arc::new(ProbeAsyncNotifyHook));
    laplace_rt::install_async_channel_hook(Arc::new(ProbeAsyncChannelHook));
    laplace_rt::install_async_broadcast_hook(Arc::new(ProbeAsyncBroadcastHook));
    laplace_rt::install_async_cell_hook(Arc::new(ProbeAsyncCellHook));
}
