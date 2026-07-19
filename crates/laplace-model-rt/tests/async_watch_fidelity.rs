// SPDX-License-Identifier: Apache-2.0
//
// `serial()`'s `std::sync::MutexGuard` is deliberately held across the W8
// `.await` points below (it is process-wide *test* serialization, not
// application state): each test's `current_thread` runtime runs exactly one
// task, so there is no other task that could contend for it, and no
// cross-thread handoff of the guard ever occurs.
#![allow(clippy::await_holding_lock)]

//! Differential fidelity gate for the `laplace_model_rt::watch` shadow seam (AXM2
//! decision doc §5.2 — watch slice, AXM2 A2-4). Mirrors
//! `tests/async_semaphore_fidelity.rs`'s gate mechanics.
//!
//! Every scenario below runs the *same* assertions against raw
//! `tokio::sync::watch` (column A) and `laplace_model_rt::watch` (column B, no
//! hook installed = passthrough) via one shared `macro_rules!` body per
//! scenario, instantiated twice. If either column's behavior deviates from
//! the shared assertions, the test fails — that is the observational
//! equivalence check. W5 in particular exists *because* its outcome is not
//! obvious from the tokio docs alone — the raw-tokio column is the ground
//! truth that forces the right answer, not a guess baked into the shadow.
//!
//! All scenarios drive tasks with manual, single-poll-at-a-time control
//! (`poll_once` below, backed by `Waker::noop()`) to remove scheduling
//! non-determinism as a variable, except W8 which real-spawns to check
//! Send-parity.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

use laplace_model_rt::{
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
async fn w1_initial_value_observed_via_borrow_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let (_tx, rx) = $ctor;
            assert_eq!(*rx.borrow(), 1u8);
        }};
    }

    scenario!(tokio::sync::watch::channel(1u8));
    scenario!(laplace_model_rt::watch::channel(1u8));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn w2_send_then_changed_ok_then_borrow_and_update_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let (tx, mut rx) = $ctor;
            tx.send(2u8).expect("w2: send succeeds");

            let mut f = Box::pin(rx.changed());
            match poll_once(f.as_mut()) {
                Poll::Ready(Ok(())) => {}
                other => panic!("w2: changed after send must resolve Ok, got {other:?}"),
            }
            drop(f);

            assert_eq!(*rx.borrow_and_update(), 2u8);
        }};
    }

    scenario!(tokio::sync::watch::channel(0u8));
    scenario!(laplace_model_rt::watch::channel(0u8));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn w3_has_changed_without_awaiting_changed_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let (tx, rx) = $ctor;
            assert!(
                matches!(rx.has_changed(), Ok(false)),
                "w3: nothing sent yet"
            );

            tx.send(5u8).expect("w3: send succeeds");
            assert!(
                matches!(rx.has_changed(), Ok(true)),
                "w3: has_changed must see the send without awaiting changed()"
            );
        }};
    }

    scenario!(tokio::sync::watch::channel(0u8));
    scenario!(laplace_model_rt::watch::channel(0u8));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn w4_every_receiver_dropped_then_send_closed_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let (tx, rx) = $ctor;
            drop(rx);

            match tx.send(9u8) {
                Err(_) => {}
                other => {
                    panic!("w4: send after every receiver drops must fail Closed, got {other:?}")
                }
            }
        }};
    }

    scenario!(tokio::sync::watch::channel(0u8));
    scenario!(laplace_model_rt::watch::channel(0u8));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn w5_sender_dropped_with_nothing_new_then_changed_closed_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let (tx, mut rx) = $ctor;
            // Nothing was ever sent, so the receiver's current value is
            // already considered seen — dropping the sender now must make
            // `changed()` resolve as closed rather than pend forever.
            drop(tx);

            let mut f = Box::pin(rx.changed());
            match poll_once(f.as_mut()) {
                Poll::Ready(Err(_)) => {}
                other => panic!(
                    "w5: changed after sender drop with nothing unseen must resolve Closed immediately, got {other:?}"
                ),
            }
        }};
    }

    scenario!(tokio::sync::watch::channel(0u8));
    scenario!(laplace_model_rt::watch::channel(0u8));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn w6_subscribe_creates_independent_receiver_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let (tx, rx1) = $ctor;
            let rx2 = tx.subscribe();

            tx.send(11u8).expect("w6: send succeeds");
            assert_eq!(*rx1.borrow(), 11u8);
            assert_eq!(*rx2.borrow(), 11u8);
        }};
    }

    scenario!(tokio::sync::watch::channel(0u8));
    scenario!(laplace_model_rt::watch::channel(0u8));
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

/// Replays a subscribe + send + changed shape with an [`AsyncChannelHook`]
/// installed and asserts the exact event sequence, including `subscribe`
/// reporting the same `endpoint_cloned(Receiver)` boundary as an explicit
/// `Receiver::clone` would.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn w7_event_stream_matches_expected_sequence() {
    let _serial = serial();

    let hook = Arc::new(RecordingAsyncChannelHook::new());
    install_async_channel_hook(hook.clone());
    reset_model_async_ids_for_model();

    {
        let (tx, mut rx1) = laplace_model_rt::watch::channel::<u8>(0);
        let rx2 = tx.subscribe();

        tx.send(1u8).expect("w7: send succeeds");

        let mut f = Box::pin(rx1.changed());
        assert!(matches!(poll_once(f.as_mut()), Poll::Ready(Ok(()))));
        drop(f);

        drop(rx2);
        drop(tx);
        drop(rx1);
    }

    assert_eq!(
        hook.drain(),
        vec![
            RecordedEvent::ChannelCreated(1, AsyncChannelKind::Watch),
            RecordedEvent::EndpointCloned(1, AsyncChannelSide::Receiver),
            RecordedEvent::OpResolved(1, 1, AsyncChannelOp::Send, AsyncChannelOutcome::Ok),
            RecordedEvent::OpResolved(1, 2, AsyncChannelOp::Changed, AsyncChannelOutcome::Ok),
            RecordedEvent::EndpointDropped(1, AsyncChannelSide::Receiver),
            RecordedEvent::EndpointDropped(1, AsyncChannelSide::Sender),
            RecordedEvent::EndpointDropped(1, AsyncChannelSide::Receiver),
        ],
        "w7 event sequence mismatch"
    );

    clear_async_channel_hook();
}

/// Send-parity: rewritten user code must keep compiling wherever the raw
/// tokio equivalent compiled — checked both by the `require_send` bound
/// below and by an actual `tokio::spawn` compiling and running.
#[tokio::test(flavor = "current_thread")]
async fn w8_send_parity_with_raw_tokio() {
    // This test allocates a real resource/op id (unlike a pure type-level
    // `require_send` check), so it must not race the other
    // event-sequence-asserting tests in this file over the shared
    // process-wide id counters.
    let _serial = serial();

    fn require_send<T: Send>() {}
    // Column A (raw tokio) — these hold by tokio's design.
    require_send::<tokio::sync::watch::Sender<u8>>();
    require_send::<tokio::sync::watch::Receiver<u8>>();
    // Column B (shadow) — must match.
    require_send::<laplace_model_rt::watch::Sender<u8>>();
    require_send::<laplace_model_rt::watch::Receiver<u8>>();
    require_send::<laplace_model_rt::watch::ModelWatchChanged<'static>>();

    let (tx, mut rx) = laplace_model_rt::watch::channel::<u8>(0);
    tokio::spawn(async move {
        let _ = tx.send(1u8);
    })
    .await
    .expect("spawned send task must not panic");

    rx.changed()
        .await
        .expect("changed after send must resolve Ok");
    assert_eq!(*rx.borrow_and_update(), 1u8);
}
