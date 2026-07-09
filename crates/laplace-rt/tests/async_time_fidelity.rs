// SPDX-License-Identifier: Apache-2.0
//
// `serial()`'s `std::sync::MutexGuard` is deliberately held across the T1-T4
// `.await` points below (it is process-wide *test* serialization, not
// application state): each test's `current_thread` runtime runs exactly one
// task, so there is no other task that could contend for it, and no
// cross-thread handoff of the guard ever occurs (mirrors
// `tests/async_mutex_fidelity.rs`'s identical allow + rationale).
#![allow(clippy::await_holding_lock)]

//! Differential fidelity gate for the AXM2 A2-4 `tokio::time` virtual-clock
//! shadow (`laplace_rt::time`).
//!
//! T1-T5 run the *same* assertions against raw `tokio::time` (column A) and
//! `laplace_rt::time` (column B, no [`AsyncTimerHook`] installed =
//! passthrough to real tokio) under `start_paused = true`, advancing virtual
//! time with `tokio::time::advance` (mirrors `start_paused`'s auto-advance
//! contract: idle-task time only moves on a real `.await`, never on a raw
//! `poll_once`, so both columns consume time identically — see
//! `tests/async_mutex_fidelity.rs`'s module doc for the same reasoning
//! applied to the Mutex slice). H1-H4 exercise hooked (virtual clock) mode
//! against a `FakeTimerHook` test double with manual single-poll control —
//! no tokio runtime is needed for the hooked path at all.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Mutex as StdMutex, MutexGuard as StdMutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use laplace_rt::{clear_async_timer_hook, install_async_timer_hook, AsyncTimerHook};

/// Serializes every test in this file. [`AsyncTimerHook`] installation and
/// the shared async resource/waiter-id allocator are process-wide global
/// state (mirrors `laplace-rt`'s own `TEST_GUARD` convention).
static TEST_GUARD: StdMutex<()> = StdMutex::new(());

fn serial() -> StdMutexGuard<'static, ()> {
    TEST_GUARD.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Polls a pinned future exactly once against a no-op waker and returns the
/// outcome.
fn poll_once<F: Future + ?Sized>(fut: Pin<&mut F>) -> Poll<F::Output> {
    let mut cx = Context::from_waker(Waker::noop());
    fut.poll(&mut cx)
}

// ===== T1-T5: unhooked differential (real tokio clock, `start_paused`) =====

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn t1_sleep_completion_order_matches() {
    let _serial = serial();
    clear_async_timer_hook();

    let short_a = tokio::time::sleep(Duration::from_millis(10));
    let long_a = tokio::time::sleep(Duration::from_millis(50));
    tokio::pin!(short_a);
    tokio::pin!(long_a);

    let short_b = laplace_rt::time::sleep(Duration::from_millis(10));
    let long_b = laplace_rt::time::sleep(Duration::from_millis(50));
    tokio::pin!(short_b);
    tokio::pin!(long_b);

    // Anchor both columns' deadlines to the same virtual instant before
    // advancing: raw tokio's `Sleep` captures its deadline eagerly at
    // construction, but the shadow's unhooked delegate is built lazily, on
    // this first poll (see the module's honesty contract — building the real
    // `tokio::time::Sleep` needs a runtime context construction may not
    // have). Without this priming poll, `short_b`/`long_b` would anchor to
    // "now" *after* the advance below instead of before it.
    assert!(poll_once(short_a.as_mut()).is_pending());
    assert!(poll_once(long_a.as_mut()).is_pending());
    assert!(poll_once(short_b.as_mut()).is_pending());
    assert!(poll_once(long_b.as_mut()).is_pending());

    tokio::time::advance(Duration::from_millis(10)).await;
    assert!(
        poll_once(short_a.as_mut()).is_ready(),
        "t1: raw short sleep must resolve after its own duration"
    );
    assert!(
        poll_once(short_b.as_mut()).is_ready(),
        "t1: shadow short sleep must resolve after its own duration"
    );
    assert!(
        poll_once(long_a.as_mut()).is_pending(),
        "t1: raw long sleep must still be pending"
    );
    assert!(
        poll_once(long_b.as_mut()).is_pending(),
        "t1: shadow long sleep must still be pending"
    );

    tokio::time::advance(Duration::from_millis(40)).await;
    assert!(
        poll_once(long_a.as_mut()).is_ready(),
        "t1: raw long sleep must resolve once its duration elapses"
    );
    assert!(
        poll_once(long_b.as_mut()).is_ready(),
        "t1: shadow long sleep must resolve once its duration elapses"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn t2_timeout_value_first_and_deadline_first_matches() {
    let _serial = serial();
    clear_async_timer_hook();

    // Value-first poll order: an immediately-ready future must win even
    // against an already-elapsed (zero-duration) deadline — this is only
    // observable because `Timeout::poll` polls the value before the timer.
    let ok_a = tokio::time::timeout(Duration::ZERO, async { 42_u8 }).await;
    let ok_b = laplace_rt::time::timeout(Duration::ZERO, async { 42_u8 }).await;
    assert_eq!(ok_a.ok(), Some(42), "t2: raw value-first case must be Ok");
    assert_eq!(
        ok_b.ok(),
        Some(42),
        "t2: shadow value-first case must be Ok"
    );

    // Deadline-first: a future that never completes must lose once its
    // duration elapses.
    let mut err_a = Box::pin(tokio::time::timeout(
        Duration::from_millis(10),
        std::future::pending::<()>(),
    ));
    let mut err_b = Box::pin(laplace_rt::time::timeout(
        Duration::from_millis(10),
        std::future::pending::<()>(),
    ));
    assert!(poll_once(err_a.as_mut()).is_pending());
    assert!(poll_once(err_b.as_mut()).is_pending());

    tokio::time::advance(Duration::from_millis(10)).await;
    let a_result = match poll_once(err_a.as_mut()) {
        Poll::Ready(r) => r,
        Poll::Pending => panic!("t2: raw timeout must resolve once its deadline elapses"),
    };
    let b_result = match poll_once(err_b.as_mut()) {
        Poll::Ready(r) => r,
        Poll::Pending => panic!("t2: shadow timeout must resolve once its deadline elapses"),
    };
    assert!(a_result.is_err(), "t2: raw deadline-first case must be Err");
    let shadow_err = b_result.expect_err("t2: shadow deadline-first case must be Err");
    assert_eq!(
        shadow_err.to_string(),
        "deadline has elapsed",
        "t2: shadow Elapsed Display must match tokio's own"
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn t3_interval_tick_spacing_matches() {
    let _serial = serial();
    clear_async_timer_hook();

    let mut a = tokio::time::interval(Duration::from_millis(10));
    let mut b = laplace_rt::time::interval(Duration::from_millis(10));

    // First tick completes immediately for both.
    a.tick().await;
    b.tick().await;

    for i in 1..=3 {
        tokio::time::advance(Duration::from_millis(10)).await;
        let mut fa = Box::pin(a.tick());
        let mut fb = Box::pin(b.tick());
        assert!(
            poll_once(fa.as_mut()).is_ready(),
            "t3: raw interval tick {i} must be ready after one period"
        );
        assert!(
            poll_once(fb.as_mut()).is_ready(),
            "t3: shadow interval tick {i} must be ready after one period"
        );
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn t4_interval_burst_catchup_matches() {
    let _serial = serial();
    clear_async_timer_hook();

    let mut a = tokio::time::interval(Duration::from_millis(10));
    let mut b = laplace_rt::time::interval(Duration::from_millis(10));
    a.tick().await;
    b.tick().await;

    // Fall behind by several periods without ticking, then observe both
    // columns fire a burst of immediately-ready ticks while catching up
    // (tokio's default `MissedTickBehavior::Burst`).
    tokio::time::advance(Duration::from_millis(35)).await;

    for i in 1..=3 {
        let mut fa = Box::pin(a.tick());
        let mut fb = Box::pin(b.tick());
        assert!(
            poll_once(fa.as_mut()).is_ready(),
            "t4: raw interval must burst-fire catch-up tick {i} immediately"
        );
        assert!(
            poll_once(fb.as_mut()).is_ready(),
            "t4: shadow interval must burst-fire catch-up tick {i} immediately"
        );
    }
}

#[test]
fn t5_send_parity_with_raw_tokio() {
    fn require_send<T: Send>() {}

    require_send::<tokio::time::Sleep>();
    require_send::<laplace_rt::time::Sleep>();

    require_send::<tokio::time::Timeout<std::future::Ready<()>>>();
    require_send::<laplace_rt::time::Timeout<std::future::Ready<()>>>();

    require_send::<tokio::time::Interval>();
    require_send::<laplace_rt::time::Interval>();
}

// ===== H1-H4: hooked (virtual clock) mode, no tokio runtime needed =====

/// Test-local [`AsyncTimerHook`] with a settable virtual clock and recorded
/// `register`/`timer_dropped` calls.
struct FakeTimerHook {
    now_nanos: StdMutex<u64>,
    registered: StdMutex<Vec<(u64, u64)>>,
    dropped: StdMutex<Vec<u64>>,
}

impl FakeTimerHook {
    fn new(now_nanos: u64) -> Self {
        Self {
            now_nanos: StdMutex::new(now_nanos),
            registered: StdMutex::new(Vec::new()),
            dropped: StdMutex::new(Vec::new()),
        }
    }

    fn set_now(&self, nanos: u64) {
        *self.now_nanos.lock().expect("now lock") = nanos;
    }

    fn registrations(&self) -> Vec<(u64, u64)> {
        self.registered.lock().expect("registered lock").clone()
    }

    fn dropped(&self) -> Vec<u64> {
        self.dropped.lock().expect("dropped lock").clone()
    }
}

impl AsyncTimerHook for FakeTimerHook {
    fn now_nanos(&self) -> u64 {
        *self.now_nanos.lock().expect("now lock")
    }

    fn register(&self, timer: u64, deadline_nanos: u64) {
        self.registered
            .lock()
            .expect("registered lock")
            .push((timer, deadline_nanos));
    }

    fn timer_dropped(&self, timer: u64) {
        self.dropped.lock().expect("dropped lock").push(timer);
    }
}

#[test]
fn h1_hooked_sleep_resolves_after_hook_advances_now() {
    let _serial = serial();
    laplace_rt::reset_model_async_ids_for_model();
    let hook = std::sync::Arc::new(FakeTimerHook::new(0));
    install_async_timer_hook(hook.clone());

    let mut fut = Box::pin(laplace_rt::time::sleep(Duration::from_nanos(100)));
    assert!(
        poll_once(fut.as_mut()).is_pending(),
        "h1: sleep must be pending before its virtual deadline"
    );
    assert_eq!(
        hook.registrations(),
        vec![(1, 100)],
        "h1: first pending poll must register (timer=1, deadline=100)"
    );

    hook.set_now(100);
    assert!(
        poll_once(fut.as_mut()).is_ready(),
        "h1: sleep must resolve once now_nanos reaches the deadline"
    );

    clear_async_timer_hook();
}

#[test]
fn h2_hooked_sleep_already_elapsed_resolves_without_register() {
    let _serial = serial();
    laplace_rt::reset_model_async_ids_for_model();
    let hook = std::sync::Arc::new(FakeTimerHook::new(500));
    install_async_timer_hook(hook.clone());

    // Mode and deadline are both decided together, at first poll (see the
    // module's honesty contract), so a nonzero-duration sleep can never
    // observe "deadline already past" on its very first poll — the deadline
    // itself is `now_at_first_poll + duration`, computed and checked within
    // that same poll. A zero-duration sleep is the one case where "deadline
    // already reached by first poll" is reachable at all: its deadline
    // *equals* `now_at_first_poll`.
    let mut fut = Box::pin(laplace_rt::time::sleep(Duration::ZERO));

    assert!(
        poll_once(fut.as_mut()).is_ready(),
        "h2: a zero-duration sleep's deadline equals now at first poll — it \
         must resolve immediately"
    );
    assert!(
        hook.registrations().is_empty(),
        "h2: an immediately-ready first poll must never call register — the \
         timer vocabulary has no `requested` boundary distinct from register"
    );

    clear_async_timer_hook();
}

#[test]
fn h3_hooked_sleep_mid_wait_drop_reports_timer_dropped() {
    let _serial = serial();
    laplace_rt::reset_model_async_ids_for_model();
    let hook = std::sync::Arc::new(FakeTimerHook::new(0));
    install_async_timer_hook(hook.clone());

    let mut fut = Box::pin(laplace_rt::time::sleep(Duration::from_nanos(100)));
    assert!(poll_once(fut.as_mut()).is_pending());

    drop(fut);

    assert_eq!(
        hook.dropped(),
        vec![1],
        "h3: dropping a pending hooked sleep must report timer_dropped(1)"
    );

    clear_async_timer_hook();
}

#[test]
fn h4_hooked_sleep_repoll_reregisters_same_timer_deadline() {
    let _serial = serial();
    laplace_rt::reset_model_async_ids_for_model();
    let hook = std::sync::Arc::new(FakeTimerHook::new(0));
    install_async_timer_hook(hook.clone());

    let mut fut = Box::pin(laplace_rt::time::sleep(Duration::from_nanos(100)));
    assert!(poll_once(fut.as_mut()).is_pending());
    assert!(
        poll_once(fut.as_mut()).is_pending(),
        "h4: a re-poll before the deadline must stay pending"
    );

    assert_eq!(
        hook.registrations(),
        vec![(1, 100), (1, 100)],
        "h4: re-polling without advancing time must re-register the exact \
         same (timer, deadline) pair — the hook's contract requires this to \
         be a cheap no-op on its side"
    );

    clear_async_timer_hook();
}
