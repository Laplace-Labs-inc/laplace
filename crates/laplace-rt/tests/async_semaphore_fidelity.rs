// SPDX-License-Identifier: Apache-2.0
//
// `serial()`'s `std::sync::MutexGuard` is deliberately held across `.await`
// points below (it is process-wide *test* serialization, not application
// state): each test's `current_thread` runtime runs exactly one task, so
// there is no other task that could contend for it, and no cross-thread
// handoff of the guard ever occurs.
#![allow(clippy::await_holding_lock)]

//! Differential fidelity gate for the `ModelAsyncSemaphore` shadow seam
//! (AXM2 decision doc §5.2 — Semaphore slice, AXM2 A2-3 slice 2). Mirrors
//! `tests/async_mutex_fidelity.rs`'s gate mechanics.
//!
//! Every scenario below runs the *same* assertions against raw
//! `tokio::sync::Semaphore` (column A) and `laplace_rt::ModelAsyncSemaphore`
//! (column B, no hook installed = passthrough) via one shared
//! `macro_rules!` body per scenario, instantiated twice. If either column's
//! behavior deviates from the shared assertions, the test fails — that is
//! the observational equivalence check.
//!
//! All scenarios drive tasks with manual, single-poll-at-a-time control
//! (`poll_once` below, backed by `Waker::noop()`) to remove scheduling
//! non-determinism as a variable.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

use laplace_rt::{
    clear_async_lock_hook, install_async_lock_hook, reset_model_async_ids_for_model,
    AsyncAcquireKind, AsyncLockHook, ModelAsyncSemaphore,
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
async fn p1_within_capacity_immediate_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let s = $ctor;

            let mut f0 = Box::pin(s.acquire());
            let p0 = match poll_once(f0.as_mut()) {
                Poll::Ready(Ok(p)) => p,
                Poll::Ready(Err(_)) => panic!("p1: semaphore must not be closed"),
                Poll::Pending => panic!("p1: acquire within capacity must resolve immediately"),
            };
            assert_eq!(s.available_permits(), 1);
            drop(p0);
            assert_eq!(s.available_permits(), 2);
        }};
    }

    scenario!(tokio::sync::Semaphore::new(2));
    scenario!(ModelAsyncSemaphore::new(2));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn p2_exhausted_then_fifo_succession_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let s = $ctor;

            let mut f0 = Box::pin(s.acquire());
            let p0 = match poll_once(f0.as_mut()) {
                Poll::Ready(Ok(p)) => p,
                _ => panic!("p2: p0 must acquire immediately"),
            };
            assert_eq!(s.available_permits(), 0);

            let mut f1 = Box::pin(s.acquire());
            assert!(
                matches!(poll_once(f1.as_mut()), Poll::Pending),
                "p2: f1 must queue once capacity is exhausted"
            );

            drop(p0);

            match poll_once(f1.as_mut()) {
                Poll::Ready(Ok(_)) => {}
                other => panic!("p2: f1 must resolve once p0 drops, got {other:?}"),
            };
        }};
    }

    scenario!(tokio::sync::Semaphore::new(1));
    scenario!(ModelAsyncSemaphore::new(1));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn p3_queued_acquire_many_blocks_later_single_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let s = $ctor;

            // Hold 1 of 2 permits, leaving 1 available.
            let mut f0 = Box::pin(s.acquire());
            let p0 = match poll_once(f0.as_mut()) {
                Poll::Ready(Ok(p)) => p,
                _ => panic!("p3: p0 must acquire immediately"),
            };
            assert_eq!(s.available_permits(), 1);

            // acquire_many(2) needs 2, only 1 is available: must queue.
            let mut f1 = Box::pin(s.acquire_many(2));
            assert!(
                matches!(poll_once(f1.as_mut()), Poll::Pending),
                "p3: acquire_many(2) must queue when only 1 permit is available"
            );

            // A later acquire(1), individually satisfiable against the 1
            // available permit, must still be blocked by FIFO fairness
            // behind the queued acquire_many(2).
            let mut f2 = Box::pin(s.acquire());
            assert!(
                matches!(poll_once(f2.as_mut()), Poll::Pending),
                "p3: a later acquire(1) must not barge a queued acquire_many(2)"
            );

            drop(p0);
        }};
    }

    scenario!(tokio::sync::Semaphore::new(2));
    scenario!(ModelAsyncSemaphore::new(2));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn p4_try_acquire_success_and_failure_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let s = $ctor;

            let p0 = s
                .try_acquire()
                .expect("p4: try_acquire succeeds within capacity");
            assert_eq!(s.available_permits(), 0);

            assert!(
                s.try_acquire().is_err(),
                "p4: try_acquire must fail once capacity is exhausted"
            );

            drop(p0);
            assert!(
                s.try_acquire().is_ok(),
                "p4: try_acquire succeeds after release"
            );
        }};
    }

    scenario!(tokio::sync::Semaphore::new(1));
    scenario!(ModelAsyncSemaphore::new(1));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn p5_forget_permanently_reduces_capacity_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let s = $ctor;

            let p0 = s
                .try_acquire()
                .expect("p5: try_acquire succeeds within capacity");
            assert_eq!(s.available_permits(), 0);
            p0.forget();

            // Capacity never returns — a fresh acquire must queue forever
            // (checked here as: still Pending after the forget).
            let mut f1 = Box::pin(s.acquire());
            assert!(
                matches!(poll_once(f1.as_mut()), Poll::Pending),
                "p5: forgotten permit must not return capacity"
            );
            assert_eq!(s.available_permits(), 0);
        }};
    }

    scenario!(tokio::sync::Semaphore::new(1));
    scenario!(ModelAsyncSemaphore::new(1));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn p6_add_permits_wakes_waiter_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let s = $ctor;

            let p0 = s
                .try_acquire()
                .expect("p6: try_acquire succeeds within capacity");
            assert_eq!(s.available_permits(), 0);

            let mut f1 = Box::pin(s.acquire());
            assert!(
                matches!(poll_once(f1.as_mut()), Poll::Pending),
                "p6: f1 must queue once capacity is exhausted"
            );

            s.add_permits(1);

            match poll_once(f1.as_mut()) {
                Poll::Ready(Ok(_)) => {}
                other => panic!("p6: f1 must resolve once add_permits(1) runs, got {other:?}"),
            };

            drop(p0);
        }};
    }

    scenario!(tokio::sync::Semaphore::new(1));
    scenario!(ModelAsyncSemaphore::new(1));
}

/// `(kind, resource, waiter, acquire_kind, extra)`, where `extra` carries
/// the `semaphore_created`/`permits_added` payload and is `0` for events
/// that don't have one.
type RecordedEvent = (&'static str, u64, u64, Option<AsyncAcquireKind>, usize);

/// Test-local [`AsyncLockHook`] that records every boundary as a
/// [`RecordedEvent`].
struct RecordingAsyncLockHook {
    events: StdMutex<Vec<RecordedEvent>>,
}

impl RecordingAsyncLockHook {
    fn new() -> Self {
        Self {
            events: StdMutex::new(Vec::new()),
        }
    }

    fn drain(&self) -> Vec<RecordedEvent> {
        std::mem::take(&mut *self.events.lock().expect("events lock"))
    }
}

impl AsyncLockHook for RecordingAsyncLockHook {
    fn requested(&self, resource: u64, waiter: u64, kind: AsyncAcquireKind) {
        self.events.lock().expect("events lock").push((
            "requested",
            resource,
            waiter,
            Some(kind),
            0,
        ));
    }

    fn acquired(&self, resource: u64, waiter: u64, kind: AsyncAcquireKind) {
        self.events.lock().expect("events lock").push((
            "acquired",
            resource,
            waiter,
            Some(kind),
            0,
        ));
    }

    fn released(&self, resource: u64, waiter: u64, kind: AsyncAcquireKind) {
        self.events.lock().expect("events lock").push((
            "released",
            resource,
            waiter,
            Some(kind),
            0,
        ));
    }

    fn waiter_dropped(&self, resource: u64, waiter: u64) {
        self.events.lock().expect("events lock").push((
            "waiter_dropped",
            resource,
            waiter,
            None,
            0,
        ));
    }

    fn semaphore_created(&self, resource: u64, permits: usize) {
        self.events.lock().expect("events lock").push((
            "semaphore_created",
            resource,
            0,
            None,
            permits,
        ));
    }

    fn permits_added(&self, resource: u64, n: usize) {
        self.events
            .lock()
            .expect("events lock")
            .push(("permits_added", resource, 0, None, n));
    }
}

/// Replays the P2 shape with an [`AsyncLockHook`] installed and asserts the
/// exact event sequence, including `semaphore_created` firing exactly once,
/// before the first boundary event, and `permits_added` appearing when
/// capacity is added.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn p7_event_stream_matches_expected_sequence() {
    let _serial = serial();

    let hook = Arc::new(RecordingAsyncLockHook::new());
    install_async_lock_hook(hook.clone());

    // `try_acquire` as the very first observed boundary must still get
    // `semaphore_created` first.
    reset_model_async_ids_for_model();
    {
        let s = ModelAsyncSemaphore::new(1);
        let p0 = s.try_acquire().expect("p7: try_acquire succeeds");
        s.add_permits(1);
        drop(p0);
    }
    assert_eq!(
        hook.drain(),
        vec![
            ("semaphore_created", 1, 0, None, 1),
            (
                "acquired",
                1,
                1,
                Some(AsyncAcquireKind::SemaphorePermits(1)),
                0
            ),
            ("permits_added", 1, 0, None, 1),
            (
                "released",
                1,
                1,
                Some(AsyncAcquireKind::SemaphorePermits(1)),
                0
            ),
        ],
        "p7 event sequence mismatch"
    );

    clear_async_lock_hook();
}

/// Send-parity: rewritten user code must keep compiling wherever the raw
/// tokio equivalent compiled — checked both by the `require_send` bound
/// below and by an actual `tokio::spawn(async { s.acquire().await })`
/// compiling and running.
#[tokio::test(flavor = "current_thread")]
async fn p8_send_parity_with_raw_tokio() {
    // This test allocates a real resource/waiter id (unlike a pure
    // type-level `require_send` check), so it must not race the other
    // event-sequence-asserting tests in this file over the shared
    // process-wide id counters.
    let _serial = serial();

    fn require_send<T: Send>() {}
    // Column A (raw tokio) — these hold by tokio's design.
    require_send::<tokio::sync::Semaphore>();
    require_send::<tokio::sync::SemaphorePermit<'static>>();
    // Column B (shadow) — must match.
    require_send::<ModelAsyncSemaphore>();
    require_send::<laplace_rt::ModelSemaphoreAcquire<'static>>();
    require_send::<laplace_rt::ModelSemaphorePermit<'static>>();

    let s = Arc::new(ModelAsyncSemaphore::new(1));
    let s2 = Arc::clone(&s);
    tokio::spawn(async move {
        let _p = s2.acquire().await;
    })
    .await
    .expect("spawned acquire task must not panic");
}
