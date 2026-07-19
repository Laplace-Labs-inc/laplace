// SPDX-License-Identifier: Apache-2.0
//
// `serial()`'s `std::sync::MutexGuard` is deliberately held across `.await`
// points below (it is process-wide *test* serialization, not application
// state): each test's `current_thread` runtime runs exactly one task, so
// there is no other task that could contend for it, and no cross-thread
// handoff of the guard ever occurs.
#![allow(clippy::await_holding_lock)]

//! Differential fidelity gate for the `ModelAsyncRwLock` shadow seam
//! (AXM2 decision doc §5.2 — RwLock slice, AXM2 A2-3 slice 2). Mirrors
//! `tests/async_mutex_fidelity.rs`'s gate mechanics.
//!
//! Every scenario below runs the *same* assertions against raw
//! `tokio::sync::RwLock` (column A) and `laplace_model_rt::ModelAsyncRwLock`
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

use laplace_model_rt::{
    clear_async_lock_hook, install_async_lock_hook, reset_model_async_ids_for_model,
    AsyncAcquireKind, AsyncLockHook, ModelAsyncRwLock,
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
async fn r1_uncontended_read_and_write_match() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let l = $ctor;

            let mut fr = Box::pin(l.read());
            let r = match poll_once(fr.as_mut()) {
                Poll::Ready(g) => g,
                Poll::Pending => panic!("r1: uncontended read must resolve immediately"),
            };
            drop(r);

            let mut fw = Box::pin(l.write());
            let w = match poll_once(fw.as_mut()) {
                Poll::Ready(g) => g,
                Poll::Pending => panic!("r1: uncontended write must resolve immediately"),
            };
            drop(w);
        }};
    }

    scenario!(tokio::sync::RwLock::new(0_u64));
    scenario!(ModelAsyncRwLock::new(0_u64));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn r2_concurrent_shared_readers_coexist_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let l = $ctor;

            let mut fr0 = Box::pin(l.read());
            let r0 = match poll_once(fr0.as_mut()) {
                Poll::Ready(g) => g,
                Poll::Pending => panic!("r2: r0 must acquire immediately"),
            };

            let mut fr1 = Box::pin(l.read());
            let r1 = match poll_once(fr1.as_mut()) {
                Poll::Ready(g) => g,
                Poll::Pending => panic!("r2: r1 must coexist with r0"),
            };

            drop(r0);
            drop(r1);
        }};
    }

    scenario!(tokio::sync::RwLock::new(0_u64));
    scenario!(ModelAsyncRwLock::new(0_u64));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn r3_queued_writer_blocks_later_readers_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let l = $ctor;

            let mut fr0 = Box::pin(l.read());
            let r0 = match poll_once(fr0.as_mut()) {
                Poll::Ready(g) => g,
                Poll::Pending => panic!("r3: r0 must acquire immediately"),
            };

            let mut fw = Box::pin(l.write());
            assert!(
                matches!(poll_once(fw.as_mut()), Poll::Pending),
                "r3: writer must queue behind live reader r0"
            );

            // A *new* reader arriving after the queued writer must also
            // block (write-starvation avoidance / fairness), not barge
            // ahead of the writer just because readers are otherwise
            // shareable.
            let mut fr1 = Box::pin(l.read());
            assert!(
                matches!(poll_once(fr1.as_mut()), Poll::Pending),
                "r3: reader arriving after a queued writer must not barge it"
            );

            drop(r0);
            assert!(
                matches!(poll_once(fr1.as_mut()), Poll::Pending),
                "r3: r1 must still be queued behind the writer"
            );

            let w = match poll_once(fw.as_mut()) {
                Poll::Ready(g) => g,
                Poll::Pending => panic!("r3: writer must resolve once r0 drops"),
            };
            assert!(
                matches!(poll_once(fr1.as_mut()), Poll::Pending),
                "r3: r1 must still be queued while the writer holds"
            );

            drop(w);
            match poll_once(fr1.as_mut()) {
                Poll::Ready(_) => {}
                Poll::Pending => panic!("r3: r1 must resolve once the writer drops"),
            };
        }};
    }

    scenario!(tokio::sync::RwLock::new(0_u64));
    scenario!(ModelAsyncRwLock::new(0_u64));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn r4_dropping_queued_writer_unblocks_reader_matches() {
    let _serial = serial();

    macro_rules! scenario {
        ($ctor:expr) => {{
            let l = $ctor;

            let mut fr0 = Box::pin(l.read());
            let r0 = match poll_once(fr0.as_mut()) {
                Poll::Ready(g) => g,
                Poll::Pending => panic!("r4: r0 must acquire immediately"),
            };

            let mut fw = Box::pin(l.write());
            assert!(
                matches!(poll_once(fw.as_mut()), Poll::Pending),
                "r4: writer must queue behind r0"
            );

            let mut fr1 = Box::pin(l.read());
            assert!(
                matches!(poll_once(fr1.as_mut()), Poll::Pending),
                "r4: r1 must queue behind the writer"
            );

            drop(r0); // handoff reserved to the writer
            drop(fw); // cancel the reserved-but-unpolled writer

            match poll_once(fr1.as_mut()) {
                Poll::Ready(_) => {}
                Poll::Pending => panic!("r4: r1 must resolve after the writer's cancellation"),
            };
        }};
    }

    scenario!(tokio::sync::RwLock::new(0_u64));
    scenario!(ModelAsyncRwLock::new(0_u64));
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
        unreachable!("RwLock-only fidelity scenarios never touch a semaphore")
    }

    fn permits_added(&self, _resource: u64, _n: usize) {
        unreachable!("RwLock-only fidelity scenarios never touch a semaphore")
    }
}

/// Replays the R2/R3 shapes with an [`AsyncLockHook`] installed and asserts
/// the exact event sequence each shape must produce. `reset` before each
/// shape makes the resource id (always the shape's single rwlock) and
/// waiter ids (allocated in read()/write() call order, starting at 1) fully
/// deterministic.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn r5_event_stream_matches_expected_sequence() {
    let _serial = serial();

    let hook = Arc::new(RecordingAsyncLockHook::new());
    install_async_lock_hook(hook.clone());

    // R2 shape: two concurrent shared readers, both immediate.
    reset_model_async_ids_for_model();
    {
        let l = ModelAsyncRwLock::new(0_u64);

        let mut fr0 = Box::pin(l.read());
        let r0 = match poll_once(fr0.as_mut()) {
            Poll::Ready(g) => g,
            Poll::Pending => panic!("r5/r2 shape: r0 must acquire immediately"),
        };

        let mut fr1 = Box::pin(l.read());
        let r1 = match poll_once(fr1.as_mut()) {
            Poll::Ready(g) => g,
            Poll::Pending => panic!("r5/r2 shape: r1 must acquire immediately"),
        };

        drop(r0);
        drop(r1);
    }
    assert_eq!(
        hook.drain(),
        vec![
            ("acquired", 1, 1, Some(AsyncAcquireKind::RwRead)),
            ("acquired", 1, 2, Some(AsyncAcquireKind::RwRead)),
            ("released", 1, 1, Some(AsyncAcquireKind::RwRead)),
            ("released", 1, 2, Some(AsyncAcquireKind::RwRead)),
        ],
        "r5/r2 shape event sequence mismatch"
    );

    // R3 shape: reader, queued writer, later reader also blocked; drain in
    // order.
    reset_model_async_ids_for_model();
    {
        let l = ModelAsyncRwLock::new(0_u64);

        let mut fr0 = Box::pin(l.read());
        let r0 = match poll_once(fr0.as_mut()) {
            Poll::Ready(g) => g,
            Poll::Pending => panic!("r5/r3 shape: r0 must acquire immediately"),
        };

        let mut fw = Box::pin(l.write());
        assert!(matches!(poll_once(fw.as_mut()), Poll::Pending));

        let mut fr1 = Box::pin(l.read());
        assert!(matches!(poll_once(fr1.as_mut()), Poll::Pending));

        drop(r0);
        let w = match poll_once(fw.as_mut()) {
            Poll::Ready(g) => g,
            Poll::Pending => panic!("r5/r3 shape: writer must resolve after r0 drops"),
        };
        drop(w);

        let r1 = match poll_once(fr1.as_mut()) {
            Poll::Ready(g) => g,
            Poll::Pending => panic!("r5/r3 shape: r1 must resolve after writer drops"),
        };
        drop(r1);
    }
    assert_eq!(
        hook.drain(),
        vec![
            ("acquired", 1, 1, Some(AsyncAcquireKind::RwRead)),
            ("requested", 1, 2, Some(AsyncAcquireKind::RwWrite)),
            ("requested", 1, 3, Some(AsyncAcquireKind::RwRead)),
            ("released", 1, 1, Some(AsyncAcquireKind::RwRead)),
            ("acquired", 1, 2, Some(AsyncAcquireKind::RwWrite)),
            ("released", 1, 2, Some(AsyncAcquireKind::RwWrite)),
            ("acquired", 1, 3, Some(AsyncAcquireKind::RwRead)),
            ("released", 1, 3, Some(AsyncAcquireKind::RwRead)),
        ],
        "r5/r3 shape event sequence mismatch"
    );

    clear_async_lock_hook();
}

/// Send-parity: rewritten user code must keep compiling wherever the raw
/// tokio equivalent compiled. `tokio::spawn` requires `Send` futures, so the
/// `read()`/`write()` futures and their guards must be `Send` exactly like
/// tokio's own — checked both by the `require_send` bound below and by an
/// actual `tokio::spawn(async { l.read().await })` compiling and running.
#[tokio::test(flavor = "current_thread")]
async fn r6_send_parity_with_raw_tokio() {
    // This test allocates a real resource/waiter id (unlike a pure
    // type-level `require_send` check), so it must not race the other
    // event-sequence-asserting tests in this file over the shared
    // process-wide id counters.
    let _serial = serial();

    fn require_send<T: Send>() {}
    // Column A (raw tokio) — these hold by tokio's design.
    require_send::<tokio::sync::RwLock<u64>>();
    require_send::<tokio::sync::RwLockReadGuard<'static, u64>>();
    require_send::<tokio::sync::RwLockWriteGuard<'static, u64>>();
    // Column B (shadow) — must match.
    require_send::<ModelAsyncRwLock<u64>>();
    require_send::<laplace_model_rt::ModelAsyncRead<'static, u64>>();
    require_send::<laplace_model_rt::ModelAsyncWrite<'static, u64>>();
    require_send::<laplace_model_rt::ModelAsyncRwLockReadGuard<'static, u64>>();
    require_send::<laplace_model_rt::ModelAsyncRwLockWriteGuard<'static, u64>>();

    let l = Arc::new(ModelAsyncRwLock::new(0_u64));
    let l2 = Arc::clone(&l);
    tokio::spawn(async move {
        let _r = l2.read().await;
    })
    .await
    .expect("spawned read task must not panic");
}
