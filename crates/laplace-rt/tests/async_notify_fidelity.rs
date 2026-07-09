// SPDX-License-Identifier: Apache-2.0
//
// `serial()`'s `std::sync::MutexGuard` is deliberately held across `.await`
// points below (it is process-wide *test* serialization, not application
// state): each test's `current_thread` runtime runs exactly one task, so
// there is no other task that could contend for it, and no cross-thread
// handoff of the guard ever occurs.
#![allow(clippy::await_holding_lock)]

//! Differential fidelity gate for the `ModelAsyncNotify` shadow seam
//! (AXM2 decision doc §5.2 — Notify slice, AXM2 A2-3 slice 2). Mirrors
//! `tests/async_mutex_fidelity.rs`'s gate mechanics.
//!
//! Every scenario below runs the *same* assertions against raw
//! `tokio::sync::Notify` (column A) and `laplace_rt::ModelAsyncNotify`
//! (column B, no hook installed = passthrough) via one shared
//! `macro_rules!` body per scenario, instantiated twice. If either column's
//! behavior deviates from the shared assertions, the test fails — that is
//! the observational equivalence check.
//!
//! All scenarios drive tasks with manual, single-poll-at-a-time control
//! (`poll_once` below, backed by `Waker::noop()`) to remove scheduling
//! non-determinism as a variable. Registration against a `Notify` happens
//! only on a `notified()` future's first poll (see each module's honesty
//! contract) — scenarios that need a waiter "registered" poll it once first
//! and check for `Poll::Pending` before proceeding.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

use laplace_rt::{
    clear_async_notify_hook, install_async_notify_hook, reset_model_async_ids_for_model,
    AsyncNotifyHook, ModelAsyncNotify,
};

/// Serializes every test in this file. See
/// `async_mutex_fidelity.rs`'s `TEST_GUARD` for the rationale — this file
/// shares the same process-wide hook/id-allocator global state.
static TEST_GUARD: StdMutex<()> = StdMutex::new(());

fn serial() -> StdMutexGuard<'static, ()> {
    TEST_GUARD.lock().unwrap_or_else(PoisonError::into_inner)
}

fn poll_once<F: Future + ?Sized>(fut: Pin<&mut F>) -> Poll<F::Output> {
    let mut cx = Context::from_waker(Waker::noop());
    fut.poll(&mut cx)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn n1_leading_notify_one_stores_permit_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let n = $ctor;

            n.notify_one();

            let mut f0 = Box::pin(n.notified());
            match poll_once(f0.as_mut()) {
                Poll::Ready(()) => {}
                Poll::Pending => panic!("n1: a leading notify_one() must be stored as a permit"),
            };
        }};
    }

    scenario!(tokio::sync::Notify::new());
    scenario!(ModelAsyncNotify::new());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn n2_permit_does_not_accumulate_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let n = $ctor;

            n.notify_one();
            n.notify_one();

            let mut f0 = Box::pin(n.notified());
            match poll_once(f0.as_mut()) {
                Poll::Ready(()) => {}
                Poll::Pending => panic!("n2: the first notified() must consume the stored permit"),
            };

            let mut f1 = Box::pin(n.notified());
            assert!(
                matches!(poll_once(f1.as_mut()), Poll::Pending),
                "n2: two notify_one() calls must not store two permits"
            );

            drop(f1);
        }};
    }

    scenario!(tokio::sync::Notify::new());
    scenario!(ModelAsyncNotify::new());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn n3_notify_one_wakes_only_the_head_waiter_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let n = $ctor;

            let mut f0 = Box::pin(n.notified());
            assert!(
                matches!(poll_once(f0.as_mut()), Poll::Pending),
                "n3: f0 must register (no stored permit yet)"
            );

            let mut f1 = Box::pin(n.notified());
            assert!(
                matches!(poll_once(f1.as_mut()), Poll::Pending),
                "n3: f1 must also register behind f0"
            );

            n.notify_one();

            match poll_once(f0.as_mut()) {
                Poll::Ready(()) => {}
                Poll::Pending => panic!("n3: f0 (head) must be woken by notify_one()"),
            };
            assert!(
                matches!(poll_once(f1.as_mut()), Poll::Pending),
                "n3: f1 must remain queued — notify_one() wakes only the head"
            );

            drop(f1);
        }};
    }

    scenario!(tokio::sync::Notify::new());
    scenario!(ModelAsyncNotify::new());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn n4_notify_waiters_wakes_only_registered_waiters_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let n = $ctor;

            let mut f0 = Box::pin(n.notified());
            assert!(
                matches!(poll_once(f0.as_mut()), Poll::Pending),
                "n4: f0 must register before notify_waiters()"
            );

            n.notify_waiters();

            match poll_once(f0.as_mut()) {
                Poll::Ready(()) => {}
                Poll::Pending => panic!("n4: registered f0 must be woken by notify_waiters()"),
            };

            // A notified() future created (and polled for the first time)
            // *after* notify_waiters() was called must not have been woken
            // by that earlier call — no permit is stored by notify_waiters.
            let mut f1 = Box::pin(n.notified());
            assert!(
                matches!(poll_once(f1.as_mut()), Poll::Pending),
                "n4: a notified() created after notify_waiters() must not see a stored permit"
            );

            drop(f1);
        }};
    }

    scenario!(tokio::sync::Notify::new());
    scenario!(ModelAsyncNotify::new());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn n5_dropping_queued_waiter_then_notify_one_succession_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let n = $ctor;

            let mut f0 = Box::pin(n.notified());
            assert!(matches!(poll_once(f0.as_mut()), Poll::Pending));

            let mut f1 = Box::pin(n.notified());
            assert!(matches!(poll_once(f1.as_mut()), Poll::Pending));

            drop(f0); // cancel the head waiter before it is ever notified

            n.notify_one();

            match poll_once(f1.as_mut()) {
                Poll::Ready(()) => {}
                Poll::Pending => {
                    panic!("n5: f1 must be woken by notify_one() after f0's cancellation")
                }
            };
        }};
    }

    scenario!(tokio::sync::Notify::new());
    scenario!(ModelAsyncNotify::new());
}

/// Test-local [`AsyncNotifyHook`] that records every boundary as
/// `(kind, resource, waiter)`. `notify_one`/`notify_waiters` carry no
/// waiter id upstream, so they are recorded with a `0` placeholder.
struct RecordingAsyncNotifyHook {
    events: StdMutex<Vec<(&'static str, u64, u64)>>,
}

impl RecordingAsyncNotifyHook {
    fn new() -> Self {
        Self {
            events: StdMutex::new(Vec::new()),
        }
    }

    fn drain(&self) -> Vec<(&'static str, u64, u64)> {
        std::mem::take(&mut *self.events.lock().expect("events lock"))
    }
}

impl AsyncNotifyHook for RecordingAsyncNotifyHook {
    fn wait_requested(&self, resource: u64, waiter: u64) {
        self.events
            .lock()
            .expect("events lock")
            .push(("wait_requested", resource, waiter));
    }

    fn wait_resolved(&self, resource: u64, waiter: u64) {
        self.events
            .lock()
            .expect("events lock")
            .push(("wait_resolved", resource, waiter));
    }

    fn notify_one(&self, resource: u64) {
        self.events
            .lock()
            .expect("events lock")
            .push(("notify_one", resource, 0));
    }

    fn notify_waiters(&self, resource: u64) {
        self.events
            .lock()
            .expect("events lock")
            .push(("notify_waiters", resource, 0));
    }

    fn waiter_dropped(&self, resource: u64, waiter: u64) {
        self.events
            .lock()
            .expect("events lock")
            .push(("waiter_dropped", resource, waiter));
    }
}

/// Replays the N1/N3 shapes with an [`AsyncNotifyHook`] installed and
/// asserts the exact event sequence each shape must produce. `reset` before
/// each shape makes the resource id (always the shape's single notify) and
/// waiter ids (allocated in `notified()` call order, starting at 1) fully
/// deterministic.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn n6_event_stream_matches_expected_sequence() {
    let _serial = serial();

    let hook = Arc::new(RecordingAsyncNotifyHook::new());
    install_async_notify_hook(hook.clone());

    // N1 shape: a leading notify_one() resolves notified() immediately — no
    // `wait_requested`, since the first poll never sees an empty permit.
    reset_model_async_ids_for_model();
    {
        let n = ModelAsyncNotify::new();
        n.notify_one();

        let mut f0 = Box::pin(n.notified());
        match poll_once(f0.as_mut()) {
            Poll::Ready(()) => {}
            Poll::Pending => panic!("n6/n1 shape: notified() must resolve immediately"),
        };
    }
    assert_eq!(
        hook.drain(),
        vec![("notify_one", 1, 0), ("wait_resolved", 1, 1)],
        "n6/n1 shape event sequence mismatch"
    );

    // N3 shape: two registered waiters, notify_one() wakes only the head.
    reset_model_async_ids_for_model();
    {
        let n = ModelAsyncNotify::new();

        let mut f0 = Box::pin(n.notified());
        assert!(matches!(poll_once(f0.as_mut()), Poll::Pending));

        let mut f1 = Box::pin(n.notified());
        assert!(matches!(poll_once(f1.as_mut()), Poll::Pending));

        n.notify_one();

        let ready0 = poll_once(f0.as_mut());
        assert!(matches!(ready0, Poll::Ready(())));

        assert!(matches!(poll_once(f1.as_mut()), Poll::Pending));
        drop(f1);
    }
    let n3_events = hook.drain();
    assert_eq!(
        n3_events,
        vec![
            ("wait_requested", 1, 1),
            ("wait_requested", 1, 2),
            ("notify_one", 1, 0),
            ("wait_resolved", 1, 1),
            ("waiter_dropped", 1, 2),
        ],
        "n6/n3 shape event sequence mismatch"
    );

    clear_async_notify_hook();
}

/// Send-parity: rewritten user code must keep compiling wherever the raw
/// tokio equivalent compiled — checked both by the `require_send` bound
/// below and by an actual `tokio::spawn(async { n.notified().await })`
/// compiling and running.
#[tokio::test(flavor = "current_thread")]
async fn n7_send_parity_with_raw_tokio() {
    // This test allocates a real resource/waiter id (unlike a pure
    // type-level `require_send` check), so it must not race the other
    // event-sequence-asserting tests in this file over the shared
    // process-wide id counters.
    let _serial = serial();

    fn require_send<T: Send>() {}
    // Column A (raw tokio) — these hold by tokio's design.
    require_send::<tokio::sync::Notify>();
    require_send::<tokio::sync::futures::Notified<'static>>();
    // Column B (shadow) — must match.
    require_send::<ModelAsyncNotify>();
    require_send::<laplace_rt::ModelNotified<'static>>();

    let n = Arc::new(ModelAsyncNotify::new());
    let n2 = Arc::clone(&n);
    let waiter = tokio::spawn(async move {
        n2.notified().await;
    });
    // Give the spawned task a chance to register before notifying it —
    // real scheduling, not manual polling, drives this test (Send-parity
    // through an actual spawn boundary is the point).
    tokio::task::yield_now().await;
    n.notify_one();
    waiter.await.expect("spawned notified task must not panic");
}
