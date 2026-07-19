// SPDX-License-Identifier: Apache-2.0
//! `tokio::time::{sleep,timeout,interval}`-compatible model virtual-clock
//! shadow.
//!
//! ## Honesty contract
//!
//! - **Mode fixed once, at the first poll.** The first poll of a [`Sleep`]/
//!   [`Timeout`]/[`Interval`] checks whether an [`crate::AsyncTimerHook`] is
//!   installed and commits to that mode (hooked or unhooked) for the rest of
//!   that future's life — a hook installed or cleared mid-flight cannot flip
//!   an already-polled timer to the other mode. The hooked mode captures the
//!   hook `Arc` itself at that first poll, so a later `clear` cannot strand
//!   an in-flight hooked timer either.
//! - **Unhooked = wrap-real, not reimplemented.** With no hook installed,
//!   each type lazily builds the real `tokio::time` equivalent at first poll
//!   (construction cannot build it eagerly — it requires a runtime context
//!   that a bare constructor call site may not have yet) and delegates every
//!   poll to it. `tests/async_time_fidelity.rs` is the differential
//!   evidence, run against raw `tokio::time` under `start_paused = true`.
//! - **Hooked = virtual, not wall-clock.** With a hook installed, deadlines
//!   are tracked in nanoseconds against [`crate::AsyncTimerHook::now_nanos`];
//!   there is no waker bookkeeping here — the engine executor drives
//!   progress by re-polling on its own step schedule, the same seam shape as
//!   the private engine's own timer seam. Resolution is nanosecond-exact,
//!   unlike real tokio's millisecond-rounded timers.
//! - **`Elapsed` is this module's own type**, not a re-export of
//!   `tokio::time::error::Elapsed` — the hooked mode has no real tokio
//!   `Elapsed` to return, only an equivalent value. `From<tokio::time::error::Elapsed>`
//!   is provided so `?`-propagation from a raw tokio call site still works.
//! - **Loud residual.** `sleep_until`, `interval_at`, `timeout_at`,
//!   `Instant`, `pause`/`advance`/`resume`, a custom `MissedTickBehavior`,
//!   and `Sleep::reset` are not provided by this module — annotated model
//!   code that references them is flagged via the `TOKIO_TIME`
//!   [`crate::unmodeled`] marker rather than silently compiling against a
//!   partial shadow. [`Interval`]'s hooked mode always mirrors tokio's
//!   default `MissedTickBehavior::Burst`.
//! - **Hooked `Interval::tick()` returns a real `Instant`, not a virtual
//!   one.** This module does not model `tokio::time::Instant` (see the loud
//!   residual above), so a hooked tick's return value is a real
//!   `Instant::now()` snapshot taken at resolution time — it does not
//!   reflect virtual time and must not be compared against other `Instant`s
//!   to reason about the virtual schedule. Code that only awaits `tick()`
//!   for pacing (the overwhelmingly common case) is unaffected.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use crate::hooks::{async_timer_hook, next_async_lock_waiter_id, AsyncTimerHook};

/// Saturating `Duration` → nanoseconds conversion (a `Duration` can in
/// principle exceed `u64::MAX` nanoseconds; the virtual clock saturates
/// rather than panicking or wrapping).
fn duration_to_nanos_saturating(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

/// Error returned by a resolved [`Timeout`] whose deadline elapsed before its
/// inner future did.
///
/// Mirrors `tokio::time::error::Elapsed` in behavior (`Display`/`Error`), but
/// is this module's own type — see the module's honesty contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elapsed(());

impl Elapsed {
    fn new() -> Self {
        Self(())
    }
}

impl std::fmt::Display for Elapsed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        "deadline has elapsed".fmt(f)
    }
}

impl std::error::Error for Elapsed {}

impl From<tokio::time::error::Elapsed> for Elapsed {
    fn from(_: tokio::time::error::Elapsed) -> Self {
        Self::new()
    }
}

/// Waits until `duration` has elapsed.
///
/// Mirrors `tokio::time::sleep`. See the module's honesty contract for the
/// hooked/unhooked mode split.
#[must_use]
pub fn sleep(duration: Duration) -> Sleep {
    Sleep {
        state: SleepState::Unstarted(duration),
        resolved: false,
    }
}

enum SleepState {
    /// Not yet polled — mode is not decided.
    Unstarted(Duration),
    /// Hooked (virtual clock) mode, decided at first poll.
    Hooked {
        timer: u64,
        deadline_nanos: u64,
        hook: Arc<dyn AsyncTimerHook>,
    },
    /// Unhooked mode: a lazily-built real `tokio::time::Sleep`.
    Unhooked(Pin<Box<tokio::time::Sleep>>),
}

/// Future returned by [`sleep`]. Compatible with `tokio::time::Sleep`'s
/// `.await` usage (this type does not expose `reset`/`deadline`/
/// `is_elapsed` — see the module's honesty contract).
pub struct Sleep {
    state: SleepState,
    resolved: bool,
}

impl Sleep {
    /// Commits to hooked or unhooked mode on first poll, leaving the state
    /// ready for the match in [`Sleep::poll`].
    fn start(&mut self) {
        // Matched by reference (not by value): `self.state` sits behind
        // `&mut self`, and `SleepState` is not `Copy` (the other variants
        // hold a `Pin<Box<_>>`/`Arc`), so a by-value match here would fail
        // to borrow-check. `Duration` itself is `Copy`, so `*duration`
        // copies out just the piece we need.
        let duration = match &self.state {
            SleepState::Unstarted(duration) => *duration,
            SleepState::Hooked { .. } | SleepState::Unhooked(_) => return,
        };
        self.state = match async_timer_hook() {
            Some(hook) => {
                let timer = next_async_lock_waiter_id();
                let deadline_nanos = hook
                    .now_nanos()
                    .saturating_add(duration_to_nanos_saturating(duration));
                SleepState::Hooked {
                    timer,
                    deadline_nanos,
                    hook,
                }
            }
            None => SleepState::Unhooked(Box::pin(tokio::time::sleep(duration))),
        };
    }
}

impl Future for Sleep {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.start();
        let this = self.get_mut();
        match &mut this.state {
            SleepState::Unstarted(_) => unreachable!("start() always resolves Unstarted"),
            SleepState::Hooked {
                timer,
                deadline_nanos,
                hook,
            } => {
                if hook.now_nanos() >= *deadline_nanos {
                    this.resolved = true;
                    Poll::Ready(())
                } else {
                    hook.register(*timer, *deadline_nanos);
                    Poll::Pending
                }
            }
            SleepState::Unhooked(inner) => {
                let poll = inner.as_mut().poll(cx);
                if poll.is_ready() {
                    this.resolved = true;
                }
                poll
            }
        }
    }
}

impl Drop for Sleep {
    fn drop(&mut self) {
        if self.resolved {
            return;
        }
        if let SleepState::Hooked { timer, hook, .. } = &self.state {
            hook.timer_dropped(*timer);
        }
    }
}

/// Requires a future to complete before `duration` has elapsed.
///
/// Mirrors `tokio::time::timeout`. Polls the wrapped future before the
/// timer on every poll (tokio's own order — see
/// `tokio-1.52.3/src/time/timeout.rs`), so a future that would complete
/// immediately always wins even against an already-elapsed deadline.
#[must_use]
pub fn timeout<F: Future>(duration: Duration, future: F) -> Timeout<F> {
    Timeout {
        future: Box::pin(future),
        delay: sleep(duration),
    }
}

/// Future returned by [`timeout`].
pub struct Timeout<F> {
    future: Pin<Box<F>>,
    delay: Sleep,
}

impl<F: Future> Future for Timeout<F> {
    type Output = Result<F::Output, Elapsed>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // `Timeout` is `Unpin` regardless of `F`: `Pin<Box<F>>` and `Sleep`
        // are both `Unpin` unconditionally, so projecting via `get_mut` is
        // sound without a manual/pin-project impl.
        let this = self.get_mut();

        // Value first, then the timer — tokio's own order (see the module
        // doc). Checking the delay first would let an already-elapsed
        // deadline race an immediately-ready future and fail it spuriously.
        if let Poll::Ready(value) = this.future.as_mut().poll(cx) {
            return Poll::Ready(Ok(value));
        }

        match Pin::new(&mut this.delay).poll(cx) {
            Poll::Ready(()) => Poll::Ready(Err(Elapsed::new())),
            Poll::Pending => Poll::Pending,
        }
    }
}

enum IntervalState {
    /// Not yet polled — mode is not decided.
    Unstarted(Duration),
    /// Hooked (virtual clock) mode, decided at first poll. Mirrors tokio's
    /// default `MissedTickBehavior::Burst`: a missed tick's next deadline is
    /// always `fired_deadline + period`, so a caller that polls long after
    /// several periods elapsed observes a burst of immediately-ready ticks
    /// until it catches up — exactly as repeated `poll_tick` calls on real
    /// tokio would.
    Hooked {
        timer: u64,
        period_nanos: u64,
        next_deadline_nanos: u64,
        hook: Arc<dyn AsyncTimerHook>,
    },
    /// Unhooked mode: a lazily-built real `tokio::time::Interval`.
    Unhooked(tokio::time::Interval),
}

/// Creates a new [`Interval`] that yields with interval of `period`. The
/// first tick completes immediately.
///
/// Mirrors `tokio::time::interval`.
///
/// # Panics
///
/// Panics if `period` is zero, exactly like `tokio::time::interval` (checked
/// eagerly here too — this assertion does not need a runtime context, unlike
/// the lazily-built real/virtual timer state below).
#[must_use]
pub fn interval(period: Duration) -> Interval {
    assert!(period > Duration::new(0, 0), "`period` must be non-zero.");
    Interval {
        state: IntervalState::Unstarted(period),
    }
}

/// Handle returned by [`interval`]. Compatible with `tokio::time::Interval`'s
/// `.tick().await` usage (this type does not expose `reset*`,
/// `missed_tick_behavior`, or `set_missed_tick_behavior` — see the module's
/// honesty contract).
pub struct Interval {
    state: IntervalState,
}

impl Interval {
    /// Completes when the next instant in the interval has been reached.
    ///
    /// Mirrors `tokio::time::Interval::tick`.
    pub async fn tick(&mut self) -> tokio::time::Instant {
        std::future::poll_fn(|cx| self.poll_tick(cx)).await
    }

    fn poll_tick(&mut self, cx: &mut Context<'_>) -> Poll<tokio::time::Instant> {
        // See `Sleep::start`: matched by reference because `IntervalState`
        // is not `Copy`, only the `Duration` payload we extract is.
        let period = match &self.state {
            IntervalState::Unstarted(period) => Some(*period),
            IntervalState::Hooked { .. } | IntervalState::Unhooked(_) => None,
        };
        if let Some(period) = period {
            self.state = match async_timer_hook() {
                Some(hook) => {
                    let timer = next_async_lock_waiter_id();
                    let now = hook.now_nanos();
                    IntervalState::Hooked {
                        timer,
                        period_nanos: duration_to_nanos_saturating(period),
                        // The first tick completes immediately, mirroring
                        // `interval_at(Instant::now(), period)`.
                        next_deadline_nanos: now,
                        hook,
                    }
                }
                None => IntervalState::Unhooked(tokio::time::interval(period)),
            };
        }

        match &mut self.state {
            IntervalState::Unstarted(_) => unreachable!("resolved just above"),
            IntervalState::Hooked {
                timer,
                period_nanos,
                next_deadline_nanos,
                hook,
            } => {
                if hook.now_nanos() >= *next_deadline_nanos {
                    *next_deadline_nanos = next_deadline_nanos.saturating_add(*period_nanos);
                    Poll::Ready(tokio::time::Instant::now())
                } else {
                    hook.register(*timer, *next_deadline_nanos);
                    Poll::Pending
                }
            }
            IntervalState::Unhooked(interval) => interval.poll_tick(cx),
        }
    }
}
