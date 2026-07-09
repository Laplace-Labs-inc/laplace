// SPDX-License-Identifier: Apache-2.0
//
// `serial()`'s `std::sync::MutexGuard` is deliberately held across the O6
// `.await` points below (it is process-wide *test* serialization, not
// application state): each test's `current_thread` runtime runs exactly one
// task, so there is no other task that could contend for it, and no
// cross-thread handoff of the guard ever occurs.
#![allow(clippy::await_holding_lock)]

//! Differential fidelity gate for the `laplace_rt::oneshot` shadow seam
//! (AXM2 decision doc §5.2 — oneshot slice, AXM2 A2-4). Mirrors
//! `tests/async_semaphore_fidelity.rs`'s gate mechanics.
//!
//! Every scenario below runs the *same* assertions against raw
//! `tokio::sync::oneshot` (column A) and `laplace_rt::oneshot` (column B, no
//! hook installed = passthrough) via one shared `macro_rules!` body per
//! scenario, instantiated twice. If either column's behavior deviates from
//! the shared assertions, the test fails — that is the observational
//! equivalence check.
//!
//! All scenarios drive tasks with manual, single-poll-at-a-time control
//! (`poll_once` below, backed by `Waker::noop()`) to remove scheduling
//! non-determinism as a variable, except O6 which real-spawns to check
//! Send-parity.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

use laplace_rt::{
    clear_async_channel_hook, install_async_channel_hook, reset_model_async_ids_for_model,
    AsyncChannelHook, AsyncChannelKind, AsyncChannelOp, AsyncChannelOutcome, AsyncChannelSide,
};

/// Serializes every test in this file. See `async_mutex_fidelity.rs`'s
/// `TEST_GUARD` for the rationale — this file shares the same process-wide
/// hook/id-allocator global state.
static TEST_GUARD: StdMutex<()> = StdMutex::new(());

fn serial() -> StdMutexGuard<'static, ()> {
    TEST_GUARD.lock().unwrap_or_else(PoisonError::into_inner)
}

fn poll_once<F: Future + ?Sized>(fut: Pin<&mut F>) -> Poll<F::Output> {
    let mut cx = Context::from_waker(Waker::noop());
    fut.poll(&mut cx)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn o1_send_then_await_ok_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let (tx, rx) = $ctor;
            tx.send(42u8).expect("o1: send before await succeeds");

            let mut f = Box::pin(rx);
            match poll_once(f.as_mut()) {
                Poll::Ready(Ok(v)) => assert_eq!(v, 42),
                other => panic!("o1: await after send must resolve Ok, got {other:?}"),
            }
        }};
    }

    scenario!(tokio::sync::oneshot::channel::<u8>());
    scenario!(laplace_rt::oneshot::channel::<u8>());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn o2_sender_dropped_then_await_closed_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let (tx, rx) = $ctor;
            drop(tx);

            let mut f = Box::pin(rx);
            match poll_once(f.as_mut()) {
                Poll::Ready(Err(_)) => {}
                other => panic!("o2: await after sender drop must resolve Closed, got {other:?}"),
            }
        }};
    }

    scenario!(tokio::sync::oneshot::channel::<u8>());
    scenario!(laplace_rt::oneshot::channel::<u8>());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn o3_receiver_dropped_then_send_returns_value_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let (tx, rx) = $ctor;
            drop(rx);

            match tx.send(7u8) {
                Err(7) => {}
                other => {
                    panic!("o3: send after receiver drop must return the value back, got {other:?}")
                }
            }
        }};
    }

    scenario!(tokio::sync::oneshot::channel::<u8>());
    scenario!(laplace_rt::oneshot::channel::<u8>());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn o4_try_recv_empty_then_send_then_ok_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let (tx, mut rx) = $ctor;

            match rx.try_recv() {
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                other => panic!("o4: try_recv before send must be Empty, got {other:?}"),
            }

            tx.send(3u8).expect("o4: send succeeds");

            match rx.try_recv() {
                Ok(v) => assert_eq!(v, 3),
                other => panic!("o4: try_recv after send must be Ok, got {other:?}"),
            }
        }};
    }

    scenario!(tokio::sync::oneshot::channel::<u8>());
    scenario!(laplace_rt::oneshot::channel::<u8>());
}

/// Every boundary [`AsyncChannelHook`] can report, recorded in call order.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RecordedEvent {
    ChannelCreated(u64, AsyncChannelKind),
    OpRequested(u64, u64, AsyncChannelOp),
    OpResolved(u64, u64, AsyncChannelOp, AsyncChannelOutcome),
    OpDropped(u64, u64),
    EndpointCloned(u64, AsyncChannelSide),
    EndpointDropped(u64, AsyncChannelSide),
    ChannelClosed(u64),
}

/// Test-local [`AsyncChannelHook`] that records every boundary as a
/// [`RecordedEvent`].
struct RecordingAsyncChannelHook {
    events: StdMutex<Vec<RecordedEvent>>,
}

impl RecordingAsyncChannelHook {
    fn new() -> Self {
        Self {
            events: StdMutex::new(Vec::new()),
        }
    }

    fn drain(&self) -> Vec<RecordedEvent> {
        std::mem::take(&mut *self.events.lock().expect("events lock"))
    }
}

impl AsyncChannelHook for RecordingAsyncChannelHook {
    fn channel_created(&self, channel: u64, kind: AsyncChannelKind) {
        self.events
            .lock()
            .expect("events lock")
            .push(RecordedEvent::ChannelCreated(channel, kind));
    }

    fn op_requested(&self, channel: u64, op: u64, kind: AsyncChannelOp) {
        self.events
            .lock()
            .expect("events lock")
            .push(RecordedEvent::OpRequested(channel, op, kind));
    }

    fn op_resolved(
        &self,
        channel: u64,
        op: u64,
        kind: AsyncChannelOp,
        outcome: AsyncChannelOutcome,
    ) {
        self.events
            .lock()
            .expect("events lock")
            .push(RecordedEvent::OpResolved(channel, op, kind, outcome));
    }

    fn op_dropped(&self, channel: u64, op: u64) {
        self.events
            .lock()
            .expect("events lock")
            .push(RecordedEvent::OpDropped(channel, op));
    }

    fn endpoint_cloned(&self, channel: u64, side: AsyncChannelSide) {
        self.events
            .lock()
            .expect("events lock")
            .push(RecordedEvent::EndpointCloned(channel, side));
    }

    fn endpoint_dropped(&self, channel: u64, side: AsyncChannelSide) {
        self.events
            .lock()
            .expect("events lock")
            .push(RecordedEvent::EndpointDropped(channel, side));
    }

    fn channel_closed(&self, channel: u64) {
        self.events
            .lock()
            .expect("events lock")
            .push(RecordedEvent::ChannelClosed(channel));
    }
}

/// Replays an all-`try_recv` shape (never awaiting the [`Receiver`] as a
/// `Future`) with an [`AsyncChannelHook`] installed and asserts the exact
/// event sequence. Note: op id `1` is consumed-but-unreported here — see
/// [`laplace_rt::oneshot`]'s honesty contract: the receive op id is
/// allocated eagerly at `channel()` time (for the await path), so an
/// all-`try_recv` flow that never awaits the `Receiver` observes its first
/// *reported* op id starting at `2`, not `1`.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn o5_event_stream_matches_expected_sequence() {
    let _serial = serial();

    let hook = Arc::new(RecordingAsyncChannelHook::new());
    install_async_channel_hook(hook.clone());
    reset_model_async_ids_for_model();

    {
        let (tx, mut rx) = laplace_rt::oneshot::channel::<u8>();

        assert!(matches!(
            rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));

        tx.send(9u8).expect("o5: send succeeds");

        assert_eq!(rx.try_recv(), Ok(9));
    }

    assert_eq!(
        hook.drain(),
        vec![
            RecordedEvent::ChannelCreated(1, AsyncChannelKind::Oneshot),
            RecordedEvent::OpResolved(1, 2, AsyncChannelOp::Recv, AsyncChannelOutcome::Empty),
            RecordedEvent::OpResolved(1, 3, AsyncChannelOp::Send, AsyncChannelOutcome::Ok),
            RecordedEvent::OpResolved(1, 4, AsyncChannelOp::Recv, AsyncChannelOutcome::Ok),
            RecordedEvent::EndpointDropped(1, AsyncChannelSide::Receiver),
        ],
        "o5 event sequence mismatch"
    );

    clear_async_channel_hook();
}

/// Send-parity: rewritten user code must keep compiling wherever the raw
/// tokio equivalent compiled — checked both by the `require_send` bound
/// below and by an actual `tokio::spawn` compiling and running.
#[tokio::test(flavor = "current_thread")]
async fn o6_send_parity_with_raw_tokio() {
    // This test allocates real resource/op ids (unlike a pure type-level
    // `require_send` check), so it must not race the other
    // event-sequence-asserting tests in this file over the shared
    // process-wide id counters.
    let _serial = serial();

    fn require_send<T: Send>() {}
    // Column A (raw tokio) — these hold by tokio's design.
    require_send::<tokio::sync::oneshot::Sender<u8>>();
    require_send::<tokio::sync::oneshot::Receiver<u8>>();
    // Column B (shadow) — must match.
    require_send::<laplace_rt::oneshot::Sender<u8>>();
    require_send::<laplace_rt::oneshot::Receiver<u8>>();

    let (tx, rx) = laplace_rt::oneshot::channel::<u8>();
    tokio::spawn(async move {
        let _ = tx.send(1u8);
    })
    .await
    .expect("spawned send task must not panic");

    assert_eq!(rx.await, Ok(1));
}
