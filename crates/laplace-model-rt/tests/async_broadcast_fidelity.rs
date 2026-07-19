// SPDX-License-Identifier: Apache-2.0
//
// Broadcast is modeled as of BCAST G4 keep (LEP-0027); these differential
// tests are the fidelity gate for the wrap-real model channel.
#![allow(clippy::await_holding_lock)]

//! Differential fidelity tests for the broadcast wrap-real model channel.
//!
//! Each semantic scenario is run against raw tokio and the wrapper. The
//! wrapper must preserve tokio's values and error payloads; hook assertions
//! cover only the additional observation vocabulary.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

use laplace_model_rt::{
    broadcast, clear_async_broadcast_hook, install_async_broadcast_hook,
    reset_model_async_ids_for_model, AsyncBroadcastHook, AsyncBroadcastOp, AsyncBroadcastOutcome,
    AsyncChannelSide,
};

static TEST_GUARD: StdMutex<()> = StdMutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    TEST_GUARD.lock().unwrap_or_else(PoisonError::into_inner)
}

fn poll_once<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    let mut context = Context::from_waker(Waker::noop());
    future.poll(&mut context)
}

#[tokio::test(flavor = "current_thread")]
async fn broadcast_values_and_send_outcome_match_tokio() {
    let _serial = serial();

    macro_rules! scenario {
        ($constructor:path) => {{
            let (sender, mut first) = $constructor(4);
            let mut second = sender.subscribe();
            assert!(matches!(sender.send(7u8), Ok(2)));
            assert_eq!(first.recv().await, Ok(7));
            assert_eq!(second.recv().await, Ok(7));
        }};
    }

    scenario!(tokio::sync::broadcast::channel::<u8>);
    scenario!(broadcast::channel::<u8>);
}

#[tokio::test(flavor = "current_thread")]
async fn broadcast_subscription_and_resubscribe_match_tokio() {
    let _serial = serial();

    macro_rules! scenario {
        ($constructor:path) => {{
            let (sender, mut first) = $constructor(4);
            assert!(matches!(sender.send(1u8), Ok(1)));
            let mut tail = sender.subscribe();
            assert!(matches!(sender.send(2u8), Ok(2)));
            assert_eq!(first.recv().await, Ok(1));
            assert_eq!(first.recv().await, Ok(2));
            assert_eq!(tail.recv().await, Ok(2));

            let mut replacement = tail.resubscribe();
            assert!(matches!(sender.send(3u8), Ok(3)));
            assert_eq!(replacement.recv().await, Ok(3));
        }};
    }

    scenario!(tokio::sync::broadcast::channel::<u8>);
    scenario!(broadcast::channel::<u8>);
}

#[tokio::test(flavor = "current_thread")]
async fn broadcast_lag_and_try_recv_errors_match_tokio() {
    let _serial = serial();

    macro_rules! scenario {
        ($constructor:path) => {{
            let (sender, mut receiver) = $constructor(2);
            assert!(matches!(sender.send(1u8), Ok(1)));
            assert!(matches!(sender.send(2u8), Ok(1)));
            assert!(matches!(sender.send(3u8), Ok(1)));
            assert!(matches!(
                receiver.try_recv(),
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(1))
            ));
            assert_eq!(receiver.try_recv(), Ok(2));
            assert_eq!(receiver.try_recv(), Ok(3));
            assert!(matches!(
                receiver.try_recv(),
                Err(tokio::sync::broadcast::error::TryRecvError::Empty)
            ));
            drop(sender);
            assert!(matches!(
                receiver.try_recv(),
                Err(tokio::sync::broadcast::error::TryRecvError::Closed)
            ));
        }};
    }

    scenario!(tokio::sync::broadcast::channel::<u8>);
    scenario!(broadcast::channel::<u8>);
}

#[tokio::test(flavor = "current_thread")]
async fn broadcast_sender_and_receiver_counts_match_tokio() {
    let _serial = serial();

    macro_rules! scenario {
        ($constructor:path) => {{
            let (sender, receiver) = $constructor(2);
            assert_eq!(sender.receiver_count(), 1);
            assert_eq!(sender.len(), 0);
            assert!(sender.is_empty());
            assert_eq!(receiver.len(), 0);
            assert!(receiver.is_empty());
            let extra = sender.subscribe();
            assert_eq!(sender.receiver_count(), 2);
            drop(extra);
            assert_eq!(sender.receiver_count(), 1);
        }};
    }

    scenario!(tokio::sync::broadcast::channel::<u8>);
    scenario!(broadcast::channel::<u8>);
}

#[tokio::test(flavor = "current_thread")]
async fn broadcast_no_receiver_send_error_and_retained_drain_match_tokio() {
    let _serial = serial();

    macro_rules! scenario {
        ($constructor:path) => {{
            let (sender, receiver) = $constructor(2);
            drop(receiver);
            assert!(sender.send(1u8).is_err());

            let (sender, mut receiver) = $constructor(2);
            assert!(matches!(sender.send(9u8), Ok(1)));
            drop(sender);
            assert_eq!(receiver.recv().await, Ok(9));
            assert_eq!(
                receiver.recv().await,
                Err(tokio::sync::broadcast::error::RecvError::Closed)
            );
        }};
    }

    scenario!(tokio::sync::broadcast::channel::<u8>);
    scenario!(broadcast::channel::<u8>);
}

#[tokio::test(flavor = "current_thread")]
async fn broadcast_recv_cancel_safety_reports_drop_and_keeps_value() {
    let _serial = serial();
    let hook = Arc::new(RecordingBroadcastHook::default());
    install_async_broadcast_hook(hook.clone());
    reset_model_async_ids_for_model();

    let (sender, mut receiver) = broadcast::channel(2);
    let mut pending = Box::pin(receiver.recv());
    assert!(matches!(poll_once(pending.as_mut()), Poll::Pending));
    drop(pending);
    assert!(matches!(sender.send(11u8), Ok(1)));
    assert_eq!(receiver.recv().await, Ok(11));

    let events = hook.events.lock().expect("events lock").clone();
    assert!(events.iter().any(|event| matches!(
        event,
        BroadcastEvent::OpRequested {
            kind: AsyncBroadcastOp::Recv,
            ..
        }
    )));
    assert!(events
        .iter()
        .any(|event| matches!(event, BroadcastEvent::OpDropped { .. })));
    clear_async_broadcast_hook();
}

#[tokio::test(flavor = "current_thread")]
async fn broadcast_hook_records_payload_and_endpoint_lifecycle() {
    let _serial = serial();
    let hook = Arc::new(RecordingBroadcastHook::default());
    install_async_broadcast_hook(hook.clone());
    reset_model_async_ids_for_model();

    let (sender, receiver) = broadcast::channel::<u8>(3);
    let sender_clone = sender.clone();
    let subscribed = sender.subscribe();
    assert!(matches!(sender.send(1), Ok(2)));
    let replacement = subscribed.resubscribe();
    drop(sender_clone);
    drop(subscribed);
    drop(replacement);
    drop(receiver);
    drop(sender);

    let events = hook.events.lock().expect("events lock").clone();
    assert!(events.iter().any(|event| matches!(
        event,
        BroadcastEvent::Created {
            resource: 1,
            capacity: 3
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        BroadcastEvent::Subscribed {
            resource: 1,
            at_seq: 0,
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        BroadcastEvent::OpResolved {
            kind: AsyncBroadcastOp::Send,
            outcome: AsyncBroadcastOutcome::Ok { receivers: 2 },
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        BroadcastEvent::OpRequested {
            kind: AsyncBroadcastOp::Send,
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        BroadcastEvent::OpResolved {
            kind: AsyncBroadcastOp::Resubscribe,
            outcome: AsyncBroadcastOutcome::Ok { receivers: 1 },
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        BroadcastEvent::EndpointCloned {
            side: AsyncChannelSide::Sender,
            receiver_id: None,
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        BroadcastEvent::EndpointCloned {
            side: AsyncChannelSide::Receiver,
            receiver_id: Some(_),
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        BroadcastEvent::EndpointDropped {
            side: AsyncChannelSide::Receiver,
            receiver_id: Some(_),
            ..
        }
    )));
    clear_async_broadcast_hook();
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BroadcastEvent {
    Created {
        resource: u64,
        capacity: usize,
    },
    Subscribed {
        resource: u64,
        receiver_id: u64,
        at_seq: u64,
    },
    OpRequested {
        resource: u64,
        op: u64,
        receiver_id: Option<u64>,
        kind: AsyncBroadcastOp,
    },
    OpResolved {
        resource: u64,
        op: u64,
        receiver_id: Option<u64>,
        kind: AsyncBroadcastOp,
        outcome: AsyncBroadcastOutcome,
    },
    OpDropped {
        resource: u64,
        op: u64,
    },
    EndpointCloned {
        resource: u64,
        side: AsyncChannelSide,
        receiver_id: Option<u64>,
    },
    EndpointDropped {
        resource: u64,
        side: AsyncChannelSide,
        receiver_id: Option<u64>,
    },
}

#[derive(Default)]
struct RecordingBroadcastHook {
    events: StdMutex<Vec<BroadcastEvent>>,
}

impl AsyncBroadcastHook for RecordingBroadcastHook {
    fn broadcast_created(&self, resource: u64, capacity: usize) {
        self.events
            .lock()
            .expect("events lock")
            .push(BroadcastEvent::Created { resource, capacity });
    }

    fn subscribed(&self, resource: u64, receiver_id: u64, at_seq: u64) {
        self.events
            .lock()
            .expect("events lock")
            .push(BroadcastEvent::Subscribed {
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
        kind: AsyncBroadcastOp,
    ) {
        self.events
            .lock()
            .expect("events lock")
            .push(BroadcastEvent::OpRequested {
                resource,
                op,
                receiver_id,
                kind,
            });
    }

    fn op_resolved(
        &self,
        resource: u64,
        op: u64,
        receiver_id: Option<u64>,
        kind: AsyncBroadcastOp,
        outcome: AsyncBroadcastOutcome,
    ) {
        self.events
            .lock()
            .expect("events lock")
            .push(BroadcastEvent::OpResolved {
                resource,
                op,
                receiver_id,
                kind,
                outcome,
            });
    }

    fn op_dropped(&self, resource: u64, op: u64) {
        self.events
            .lock()
            .expect("events lock")
            .push(BroadcastEvent::OpDropped { resource, op });
    }

    fn endpoint_cloned(&self, resource: u64, side: AsyncChannelSide, receiver_id: Option<u64>) {
        self.events
            .lock()
            .expect("events lock")
            .push(BroadcastEvent::EndpointCloned {
                resource,
                side,
                receiver_id,
            });
    }

    fn endpoint_dropped(&self, resource: u64, side: AsyncChannelSide, receiver_id: Option<u64>) {
        self.events
            .lock()
            .expect("events lock")
            .push(BroadcastEvent::EndpointDropped {
                resource,
                side,
                receiver_id,
            });
    }
}
