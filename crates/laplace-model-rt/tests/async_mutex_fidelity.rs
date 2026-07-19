// SPDX-License-Identifier: Apache-2.0
//
// `serial()`'s `std::sync::MutexGuard` is deliberately held across the S4/S5
// `.await` points below (it is process-wide *test* serialization, not
// application state): each test's `current_thread` runtime runs exactly one
// task, so there is no other task that could contend for it, and no
// cross-thread handoff of the guard ever occurs.
#![allow(clippy::await_holding_lock)]

//! Differential fidelity gate for the `ModelAsyncMutex` shadow seam
//! (AXM2 decision doc §5.2 — Mutex slice, AXM2 A2-3 slice 1).
//!
//! Every scenario below runs the *same* assertions against raw
//! `tokio::sync::Mutex` (column A) and `laplace_model_rt::ModelAsyncMutex` (column
//! B, no hook installed = passthrough) via one shared `macro_rules!` body
//! per scenario, instantiated twice. If either column's behavior deviates
//! from the shared assertions, the test fails — that is the observational
//! equivalence check. Verified against tokio 1.42 (pinned via `Cargo.lock`
//! at the time this gate was authored).
//!
//! All scenarios drive a single task with manual, single-poll-at-a-time
//! control (`poll_once` below, backed by `Waker::noop()`) to remove
//! scheduling non-determinism as a variable — except S4/S5, which real-await
//! `tokio::time::timeout` under `start_paused = true` so the runtime itself
//! can observe/advance virtual time.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use laplace_model_rt::{
    clear_async_lock_hook, install_async_lock_hook, reset_model_async_ids_for_model,
    AsyncAcquireKind, AsyncLockHook, ModelAsyncMutex,
};

/// Serializes every test in this file. [`AsyncLockHook`] installation and
/// the model async-mutex resource/waiter-id allocators are process-wide
/// global state; without serialization, one test's hook or reset could
/// pollute another's concurrently-running assertions (mirrors
/// `laplace-model-rt`'s own `TEST_GUARD` unit-test convention).
static TEST_GUARD: StdMutex<()> = StdMutex::new(());

/// Acquires the serialization guard, recovering from a poisoned guard left
/// by an unrelated panicking test.
fn serial() -> StdMutexGuard<'static, ()> {
    TEST_GUARD.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Polls a pinned future exactly once against a no-op waker and returns the
/// outcome. Every scenario below re-polls only futures that have not yet
/// reported `Ready` — polling an already-completed tokio future panics, so
/// scenarios track that themselves rather than relying on this helper to
/// guard it.
fn poll_once<F: Future + ?Sized>(fut: Pin<&mut F>) -> Poll<F::Output> {
    let mut cx = Context::from_waker(Waker::noop());
    fut.poll(&mut cx)
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn s1_uncontended_immediate_acquire_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let m = $ctor;

            let mut f0 = Box::pin(m.lock());
            let g0 = match poll_once(f0.as_mut()) {
                Poll::Ready(g) => g,
                Poll::Pending => panic!("s1: uncontended lock must resolve immediately"),
            };
            drop(g0);

            let mut f1 = Box::pin(m.lock());
            match poll_once(f1.as_mut()) {
                Poll::Ready(_) => {}
                Poll::Pending => panic!("s1: reacquire after release must resolve immediately"),
            };
        }};
    }

    scenario!(tokio::sync::Mutex::new(0_u64));
    scenario!(ModelAsyncMutex::new(0_u64));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn s2_fifo_no_barge_head_only_handoff_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let m = $ctor;

            let mut f0 = Box::pin(m.lock());
            let g0 = match poll_once(f0.as_mut()) {
                Poll::Ready(g) => g,
                Poll::Pending => panic!("s2: g0 must acquire immediately"),
            };

            let mut f1 = Box::pin(m.lock());
            assert!(
                matches!(poll_once(f1.as_mut()), Poll::Pending),
                "s2: f1 must queue behind g0"
            );

            let mut f2 = Box::pin(m.lock());
            assert!(
                matches!(poll_once(f2.as_mut()), Poll::Pending),
                "s2: f2 must queue behind f1"
            );

            drop(g0);

            assert!(
                matches!(poll_once(f2.as_mut()), Poll::Pending),
                "s2: f2 must not barge ahead of f1 on handoff"
            );

            let g1 = match poll_once(f1.as_mut()) {
                Poll::Ready(g) => g,
                Poll::Pending => panic!("s2: f1 must resolve once handed off"),
            };

            assert!(
                matches!(poll_once(f2.as_mut()), Poll::Pending),
                "s2: f2 must still be queued while g1 is held"
            );

            drop(g1);

            match poll_once(f2.as_mut()) {
                Poll::Ready(_) => {}
                Poll::Pending => panic!("s2: f2 must resolve once g1 is dropped"),
            };
        }};
    }

    scenario!(tokio::sync::Mutex::new(0_u64));
    scenario!(ModelAsyncMutex::new(0_u64));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn s3_cancellation_succession_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let m = $ctor;

            let mut f0 = Box::pin(m.lock());
            let g0 = match poll_once(f0.as_mut()) {
                Poll::Ready(g) => g,
                Poll::Pending => panic!("s3: g0 must acquire immediately"),
            };

            let mut f1 = Box::pin(m.lock());
            assert!(
                matches!(poll_once(f1.as_mut()), Poll::Pending),
                "s3: f1 must queue behind g0"
            );

            let mut f2 = Box::pin(m.lock());
            assert!(
                matches!(poll_once(f2.as_mut()), Poll::Pending),
                "s3: f2 must queue behind f1"
            );

            drop(g0); // handoff reserved to f1
            drop(f1); // cancel the reserved-but-unpolled waiter

            match poll_once(f2.as_mut()) {
                Poll::Ready(_) => {}
                Poll::Pending => {
                    panic!("s3: f2 must resolve after f1's cancellation succession")
                }
            };
        }};
    }

    scenario!(tokio::sync::Mutex::new(0_u64));
    scenario!(ModelAsyncMutex::new(0_u64));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn s4_futurelock_hang_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let m = $ctor;

            let mut f0 = Box::pin(m.lock());
            let g0 = match poll_once(f0.as_mut()) {
                Poll::Ready(g) => g,
                Poll::Pending => panic!("s4: g0 must acquire immediately"),
            };

            let mut f1 = Box::pin(m.lock());
            assert!(
                matches!(poll_once(f1.as_mut()), Poll::Pending),
                "s4: f1 must queue behind g0"
            );

            drop(g0); // handoff reserved to f1, which is left alive and un-polled

            let result = tokio::time::timeout(Duration::from_secs(1), m.lock()).await;
            assert!(
                result.is_err(),
                "s4: a reserved-but-unpolled f1 must starve a fresh lock() attempt (RFD 609 futurelock)"
            );

            drop(f1);
        }};
    }

    scenario!(tokio::sync::Mutex::new(0_u64));
    scenario!(ModelAsyncMutex::new(0_u64));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn s5_dropping_the_reserved_waiter_unblocks_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let m = $ctor;

            let mut f0 = Box::pin(m.lock());
            let g0 = match poll_once(f0.as_mut()) {
                Poll::Ready(g) => g,
                Poll::Pending => panic!("s5: g0 must acquire immediately"),
            };

            let mut f1 = Box::pin(m.lock());
            assert!(
                matches!(poll_once(f1.as_mut()), Poll::Pending),
                "s5: f1 must queue behind g0"
            );

            drop(g0); // handoff reserved to f1
            drop(f1); // cancel before the timeout attempt is even created

            let result = tokio::time::timeout(Duration::from_secs(1), m.lock()).await;
            assert!(
                result.is_ok(),
                "s5: dropping the reserved-but-unpolled waiter must free the mutex"
            );
        }};
    }

    scenario!(tokio::sync::Mutex::new(0_u64));
    scenario!(ModelAsyncMutex::new(0_u64));
}

/// `(kind, resource, waiter, acquire_kind)`. `requested`/`acquired`/
/// `released` carry a real [`AsyncAcquireKind`]; `waiter_dropped` carries
/// `None`.
type RecordedEvent = (&'static str, u64, u64, Option<AsyncAcquireKind>);

/// Test-local [`AsyncLockHook`] that records every boundary as a
/// [`RecordedEvent`]. `semaphore_created`/`permits_added` are unreachable
/// here — this file never touches a semaphore.
struct RecordingAsyncLockHook {
    events: StdMutex<Vec<RecordedEvent>>,
}

impl RecordingAsyncLockHook {
    fn new() -> Self {
        Self {
            events: StdMutex::new(Vec::new()),
        }
    }

    /// Returns the events recorded so far and clears the buffer, so each
    /// scenario shape below asserts only its own events.
    fn drain(&self) -> Vec<RecordedEvent> {
        std::mem::take(&mut *self.events.lock().expect("events lock"))
    }
}

impl AsyncLockHook for RecordingAsyncLockHook {
    fn requested(&self, resource: u64, waiter: u64, kind: AsyncAcquireKind) {
        self.events
            .lock()
            .expect("events lock")
            .push(("requested", resource, waiter, Some(kind)));
    }

    fn acquired(&self, resource: u64, waiter: u64, kind: AsyncAcquireKind) {
        self.events
            .lock()
            .expect("events lock")
            .push(("acquired", resource, waiter, Some(kind)));
    }

    fn released(&self, resource: u64, waiter: u64, kind: AsyncAcquireKind) {
        self.events
            .lock()
            .expect("events lock")
            .push(("released", resource, waiter, Some(kind)));
    }

    fn waiter_dropped(&self, resource: u64, waiter: u64) {
        self.events
            .lock()
            .expect("events lock")
            .push(("waiter_dropped", resource, waiter, None));
    }

    fn semaphore_created(&self, _resource: u64, _permits: usize) {
        unreachable!("Mutex-only fidelity scenarios never touch a semaphore")
    }

    fn permits_added(&self, _resource: u64, _n: usize) {
        unreachable!("Mutex-only fidelity scenarios never touch a semaphore")
    }
}

/// Replays the S1/S2/S3 shapes with an [`AsyncLockHook`] installed and
/// asserts the exact event sequence each shape must produce. `reset` before
/// each shape makes the resource id (always the shape's single mutex) and
/// waiter ids (allocated in `.lock()` call order, starting at 1) fully
/// deterministic, so the assertions below can use concrete ids instead of
/// depending on cross-test allocation order.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn s6_event_stream_matches_expected_sequence() {
    let _serial = serial();

    let hook = Arc::new(RecordingAsyncLockHook::new());
    install_async_lock_hook(hook.clone());

    // S1 shape: a single uncontended lock + release — no `requested` event,
    // since the first poll never sees contention.
    reset_model_async_ids_for_model();
    {
        let m = ModelAsyncMutex::new(0_u64);
        let mut f0 = Box::pin(m.lock());
        let g0 = match poll_once(f0.as_mut()) {
            Poll::Ready(g) => g,
            Poll::Pending => panic!("s6/s1 shape: g0 must acquire immediately"),
        };
        drop(g0);
    }
    assert_eq!(
        hook.drain(),
        vec![
            ("acquired", 1, 1, Some(AsyncAcquireKind::Mutex)),
            ("released", 1, 1, Some(AsyncAcquireKind::Mutex)),
        ],
        "s6/s1 shape event sequence mismatch"
    );

    // S2 shape: FIFO handoff, draining all three guards through to release.
    reset_model_async_ids_for_model();
    {
        let m = ModelAsyncMutex::new(0_u64);

        let mut f0 = Box::pin(m.lock());
        let g0 = match poll_once(f0.as_mut()) {
            Poll::Ready(g) => g,
            Poll::Pending => panic!("s6/s2 shape: g0 must acquire immediately"),
        };

        let mut f1 = Box::pin(m.lock());
        assert!(matches!(poll_once(f1.as_mut()), Poll::Pending));

        let mut f2 = Box::pin(m.lock());
        assert!(matches!(poll_once(f2.as_mut()), Poll::Pending));

        drop(g0);
        assert!(matches!(poll_once(f2.as_mut()), Poll::Pending));

        let g1 = match poll_once(f1.as_mut()) {
            Poll::Ready(g) => g,
            Poll::Pending => panic!("s6/s2 shape: f1 must resolve after handoff"),
        };
        assert!(matches!(poll_once(f2.as_mut()), Poll::Pending));

        drop(g1);

        let g2 = match poll_once(f2.as_mut()) {
            Poll::Ready(g) => g,
            Poll::Pending => panic!("s6/s2 shape: f2 must resolve after g1 drops"),
        };
        drop(g2);
    }
    assert_eq!(
        hook.drain(),
        vec![
            ("acquired", 1, 1, Some(AsyncAcquireKind::Mutex)),
            ("requested", 1, 2, Some(AsyncAcquireKind::Mutex)),
            ("requested", 1, 3, Some(AsyncAcquireKind::Mutex)),
            ("released", 1, 1, Some(AsyncAcquireKind::Mutex)),
            ("acquired", 1, 2, Some(AsyncAcquireKind::Mutex)),
            ("released", 1, 2, Some(AsyncAcquireKind::Mutex)),
            ("acquired", 1, 3, Some(AsyncAcquireKind::Mutex)),
            ("released", 1, 3, Some(AsyncAcquireKind::Mutex)),
        ],
        "s6/s2 shape event sequence mismatch"
    );

    // S3 shape: cancellation succession — f1 is dropped while reserved (but
    // un-polled), and f2 must still resolve.
    reset_model_async_ids_for_model();
    {
        let m = ModelAsyncMutex::new(0_u64);

        let mut f0 = Box::pin(m.lock());
        let g0 = match poll_once(f0.as_mut()) {
            Poll::Ready(g) => g,
            Poll::Pending => panic!("s6/s3 shape: g0 must acquire immediately"),
        };

        let mut f1 = Box::pin(m.lock());
        assert!(matches!(poll_once(f1.as_mut()), Poll::Pending));

        let mut f2 = Box::pin(m.lock());
        assert!(matches!(poll_once(f2.as_mut()), Poll::Pending));

        drop(g0);
        drop(f1);

        let g2 = match poll_once(f2.as_mut()) {
            Poll::Ready(g) => g,
            Poll::Pending => panic!("s6/s3 shape: f2 must resolve after f1's succession cancel"),
        };
        drop(g2);
    }
    let s3_events = hook.drain();
    assert!(
        s3_events.contains(&("waiter_dropped", 1, 2, None)),
        "s6/s3 shape missing waiter_dropped(w1): {s3_events:?}"
    );
    assert!(
        s3_events.contains(&("acquired", 1, 3, Some(AsyncAcquireKind::Mutex))),
        "s6/s3 shape missing acquired(w2): {s3_events:?}"
    );

    clear_async_lock_hook();
}

/// Send-parity: rewritten user code must keep compiling wherever the raw
/// tokio equivalent compiled. `tokio::spawn` requires `Send` futures, so the
/// `lock()` future and guard must be `Send` exactly like tokio's own.
#[test]
fn s7_send_parity_with_raw_tokio() {
    fn require_send<T: Send>() {}
    // Column A (raw tokio) — these hold by tokio's design.
    require_send::<tokio::sync::Mutex<u64>>();
    require_send::<tokio::sync::MutexGuard<'static, u64>>();
    // Column B (shadow) — must match.
    require_send::<ModelAsyncMutex<u64>>();
    require_send::<laplace_model_rt::ModelAsyncLock<'static, u64>>();
    require_send::<laplace_model_rt::ModelAsyncMutexGuard<'static, u64>>();
}

/// `const_new` behavioral parity: a `static` built with `const_new` must
/// compile (proving the lazy-id constructor is usable in a `const` context,
/// unlike `new`) and behave identically to `new` under `.lock()` — and its
/// resource id must not be allocated until the first hook-observed
/// boundary, since `const_new` cannot run an allocator at compile time.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn s8_const_new_matches_new_and_allocates_lazily() {
    let _serial = serial();

    static M: ModelAsyncMutex<u64> = ModelAsyncMutex::const_new(0);

    let hook = Arc::new(RecordingAsyncLockHook::new());
    install_async_lock_hook(hook.clone());
    reset_model_async_ids_for_model();

    let mut f0 = Box::pin(M.lock());
    let g0 = match poll_once(f0.as_mut()) {
        Poll::Ready(g) => g,
        Poll::Pending => panic!("s8: const_new mutex must acquire immediately when uncontended"),
    };
    drop(g0);

    // The lazily-allocated resource id lands on "1" (the first id after
    // reset), exactly like an eagerly-constructed `ModelAsyncMutex::new`
    // would get — proving allocation happened exactly once, on first use,
    // not at `const_new` time (there is no such time to allocate at).
    assert_eq!(
        hook.drain(),
        vec![
            ("acquired", 1, 1, Some(AsyncAcquireKind::Mutex)),
            ("released", 1, 1, Some(AsyncAcquireKind::Mutex)),
        ],
        "s8: const_new event sequence must match new's"
    );

    clear_async_lock_hook();
}
