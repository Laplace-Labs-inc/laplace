// SPDX-License-Identifier: Apache-2.0
//
// `serial()`'s `std::sync::MutexGuard` is deliberately held across the M9
// `.await` points below (it is process-wide *test* serialization, not
// application state): each test's `current_thread` runtime runs exactly one
// task, so there is no other task that could contend for it, and no
// cross-thread handoff of the guard ever occurs.
#![allow(clippy::await_holding_lock)]

//! Differential fidelity gate for the `laplace_rt::mpsc` shadow seam (AXM2
//! decision doc §5.2 — mpsc slice, AXM2 A2-4). Mirrors
//! `tests/async_semaphore_fidelity.rs`'s gate mechanics.
//!
//! Every scenario below runs the *same* assertions against raw
//! `tokio::sync::mpsc` (column A) and `laplace_rt::mpsc` (column B, no hook
//! installed = passthrough) via one shared `macro_rules!` body per scenario,
//! instantiated twice. If either column's behavior deviates from the shared
//! assertions, the test fails — that is the observational equivalence
//! check.
//!
//! All scenarios drive tasks with manual, single-poll-at-a-time control
//! (`poll_once` below, backed by `Waker::noop()`) to remove scheduling
//! non-determinism as a variable, except M9 which real-spawns to check
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
async fn m1_uncontended_send_then_recv_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let (tx, mut rx) = $ctor;

            let mut f0 = Box::pin(tx.send(7u8));
            match poll_once(f0.as_mut()) {
                Poll::Ready(Ok(())) => {}
                other => panic!("m1: send within capacity must resolve immediately, got {other:?}"),
            }

            let mut f1 = Box::pin(rx.recv());
            match poll_once(f1.as_mut()) {
                Poll::Ready(Some(v)) => assert_eq!(v, 7),
                other => panic!("m1: recv must return the sent value immediately, got {other:?}"),
            }
        }};
    }

    scenario!(tokio::sync::mpsc::channel::<u8>(4));
    scenario!(laplace_rt::mpsc::channel::<u8>(4));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m2_backpressure_send_queues_then_recv_unblocks_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let (tx, mut rx) = $ctor;

            let mut f0 = Box::pin(tx.send(1u8));
            match poll_once(f0.as_mut()) {
                Poll::Ready(Ok(())) => {}
                other => {
                    panic!("m2: f0 must fill the single-slot buffer immediately, got {other:?}")
                }
            }

            let mut f1 = Box::pin(tx.send(2u8));
            assert!(
                matches!(poll_once(f1.as_mut()), Poll::Pending),
                "m2: f1 must queue once capacity is exhausted"
            );

            let mut fr = Box::pin(rx.recv());
            match poll_once(fr.as_mut()) {
                Poll::Ready(Some(v)) => assert_eq!(v, 1),
                other => panic!("m2: recv must return the first buffered value, got {other:?}"),
            }

            match poll_once(f1.as_mut()) {
                Poll::Ready(Ok(())) => {}
                other => panic!("m2: f1 must resolve once recv frees capacity, got {other:?}"),
            }
        }};
    }

    scenario!(tokio::sync::mpsc::channel::<u8>(1));
    scenario!(laplace_rt::mpsc::channel::<u8>(1));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m3_try_send_full_and_try_recv_empty_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let (tx, mut rx) = $ctor;

            match rx.try_recv() {
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
                other => panic!("m3: try_recv on an empty channel must be Empty, got {other:?}"),
            }

            tx.try_send(1u8)
                .expect("m3: try_send within capacity succeeds");
            match tx.try_send(2u8) {
                Err(tokio::sync::mpsc::error::TrySendError::Full(2)) => {}
                other => panic!("m3: try_send over capacity must be Full, got {other:?}"),
            }
        }};
    }

    scenario!(tokio::sync::mpsc::channel::<u8>(1));
    scenario!(laplace_rt::mpsc::channel::<u8>(1));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m4_all_senders_dropped_then_recv_none_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let (tx, mut rx) = $ctor;
            drop(tx);

            let mut f0 = Box::pin(rx.recv());
            match poll_once(f0.as_mut()) {
                Poll::Ready(None) => {}
                other => {
                    panic!("m4: recv must resolve None once every sender drops, got {other:?}")
                }
            }
        }};
    }

    scenario!(tokio::sync::mpsc::channel::<u8>(1));
    scenario!(laplace_rt::mpsc::channel::<u8>(1));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m5_receiver_close_then_send_closed_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let (tx, mut rx) = $ctor;
            rx.close();

            let mut f0 = Box::pin(tx.send(1u8));
            match poll_once(f0.as_mut()) {
                Poll::Ready(Err(tokio::sync::mpsc::error::SendError(1))) => {}
                other => panic!("m5: send after close must fail Closed, got {other:?}"),
            }
        }};
    }

    scenario!(tokio::sync::mpsc::channel::<u8>(1));
    scenario!(laplace_rt::mpsc::channel::<u8>(1));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m6_unbounded_send_immediate_then_recv_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let (tx, mut rx) = $ctor;
            tx.send(9u8).expect("m6: unbounded send never blocks");

            let mut f0 = Box::pin(rx.recv());
            match poll_once(f0.as_mut()) {
                Poll::Ready(Some(v)) => assert_eq!(v, 9),
                other => panic!("m6: recv must return the sent value, got {other:?}"),
            }
        }};
    }

    scenario!(tokio::sync::mpsc::unbounded_channel::<u8>());
    scenario!(laplace_rt::mpsc::unbounded_channel::<u8>());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m7_recv_future_dropped_mid_wait_loses_nothing_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let (tx, mut rx) = $ctor;

            let mut f0 = Box::pin(rx.recv());
            assert!(
                matches!(poll_once(f0.as_mut()), Poll::Pending),
                "m7: recv on an empty channel must queue"
            );
            drop(f0);

            let mut fsend = Box::pin(tx.send(5u8));
            match poll_once(fsend.as_mut()) {
                Poll::Ready(Ok(())) => {}
                other => panic!(
                    "m7: send after dropping the pending recv future must still succeed, got {other:?}"
                ),
            }

            let mut f1 = Box::pin(rx.recv());
            match poll_once(f1.as_mut()) {
                Poll::Ready(Some(v)) => assert_eq!(v, 5),
                other => panic!("m7: a later recv must still receive the value, got {other:?}"),
            }
        }};
    }

    scenario!(tokio::sync::mpsc::channel::<u8>(1));
    scenario!(laplace_rt::mpsc::channel::<u8>(1));
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

/// Replays a small bounded-channel shape with an [`AsyncChannelHook`]
/// installed and asserts the exact event sequence, including
/// `channel_created` firing exactly once at construction, an `op_requested`
/// for the queued second send, and both endpoints' drops.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m8_event_stream_matches_expected_sequence() {
    let _serial = serial();

    let hook = Arc::new(RecordingAsyncChannelHook::new());
    install_async_channel_hook(hook.clone());
    reset_model_async_ids_for_model();

    {
        let (tx, mut rx) = laplace_rt::mpsc::channel::<u8>(1);

        let mut f0 = Box::pin(tx.send(1u8));
        assert!(matches!(poll_once(f0.as_mut()), Poll::Ready(Ok(()))));

        let mut f1 = Box::pin(tx.send(2u8));
        assert!(matches!(poll_once(f1.as_mut()), Poll::Pending));

        let mut fr = Box::pin(rx.recv());
        assert!(matches!(poll_once(fr.as_mut()), Poll::Ready(Some(1))));

        assert!(matches!(poll_once(f1.as_mut()), Poll::Ready(Ok(()))));

        // Every future has already resolved, so dropping them reports no
        // further boundary — drop them first so `tx`/`rx` are free to move.
        drop(f0);
        drop(f1);
        drop(fr);

        drop(tx);
        drop(rx);
    }

    assert_eq!(
        hook.drain(),
        vec![
            RecordedEvent::ChannelCreated(1, AsyncChannelKind::MpscBounded { capacity: 1 }),
            RecordedEvent::OpResolved(1, 1, AsyncChannelOp::Send, AsyncChannelOutcome::Ok),
            RecordedEvent::OpRequested(1, 2, AsyncChannelOp::Send),
            RecordedEvent::OpResolved(1, 3, AsyncChannelOp::Recv, AsyncChannelOutcome::Ok),
            RecordedEvent::OpResolved(1, 2, AsyncChannelOp::Send, AsyncChannelOutcome::Ok),
            RecordedEvent::EndpointDropped(1, AsyncChannelSide::Sender),
            RecordedEvent::EndpointDropped(1, AsyncChannelSide::Receiver),
        ],
        "m8 event sequence mismatch"
    );

    clear_async_channel_hook();
}

/// Send-parity: rewritten user code must keep compiling wherever the raw
/// tokio equivalent compiled — checked both by the `require_send` bound
/// below and by an actual `tokio::spawn` compiling and running.
#[tokio::test(flavor = "current_thread")]
async fn m9_send_parity_with_raw_tokio() {
    // This test allocates real resource/op ids (unlike a pure type-level
    // `require_send` check), so it must not race the other
    // event-sequence-asserting tests in this file over the shared
    // process-wide id counters.
    let _serial = serial();

    fn require_send<T: Send>() {}
    // Column A (raw tokio) — these hold by tokio's design.
    require_send::<tokio::sync::mpsc::Sender<u8>>();
    require_send::<tokio::sync::mpsc::Receiver<u8>>();
    // Column B (shadow) — must match.
    require_send::<laplace_rt::mpsc::Sender<u8>>();
    require_send::<laplace_rt::mpsc::Receiver<u8>>();
    require_send::<laplace_rt::mpsc::ModelMpscSend<'static, u8>>();
    require_send::<laplace_rt::mpsc::ModelMpscRecv<'static, u8>>();

    let (tx, mut rx) = laplace_rt::mpsc::channel::<u8>(1);
    let tx2 = tx.clone();
    tokio::spawn(async move {
        let _ = tx2.send(1u8).await;
    })
    .await
    .expect("spawned send task must not panic");

    assert_eq!(rx.recv().await, Some(1));
}
