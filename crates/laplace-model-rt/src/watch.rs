// SPDX-License-Identifier: Apache-2.0
//! `tokio::sync::watch`-compatible model channel.
//!
//! ## Honesty contract
//!
//! - **Wrap-real, not reimplemented.** [`Sender`]/[`Receiver`] each hold a
//!   real `tokio::sync::watch` endpoint and delegate every operation to it;
//!   the semantics observed here — last-value-wins, change notification —
//!   are tokio's own, not a model reconstruction.
//! - **Differential fidelity gate**: `tests/async_watch_fidelity.rs` runs
//!   identical scenarios against raw `tokio::sync::watch` and against this
//!   wrapper and asserts observationally equivalent outcomes.
//! - **Errors are tokio's own types**, not a model reconstruction —
//!   [`tokio::sync::watch::error::SendError`] and
//!   [`tokio::sync::watch::error::RecvError`] are returned unmodified.
//! - **`borrow`/`borrow_and_update` are observation limits, by design.**
//!   [`Ref`] is a thin `Deref`-only wrapper around
//!   `tokio::sync::watch::Ref` and neither call reports a hook boundary —
//!   they read the latest value under tokio's own read lock without
//!   participating in a wait cycle, so there is no acquire/release boundary
//!   for this seam to observe.
//! - **Loud residual (AXM2 A2-4 scope cut).** `send_modify`,
//!   `send_if_modified`, `send_replace`, `wait_for`, `mark_changed`,
//!   `mark_unchanged`, and `Sender::closed` are not provided by this
//!   wrapper — calling them on model code fails loudly at compile time (no
//!   such method), not silently at runtime.

use std::future::Future;
use std::ops::Deref;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::sync::watch::error::{RecvError, SendError};

use crate::hooks::{
    async_channel_hook, next_async_lock_resource_id, next_async_lock_waiter_id, AsyncChannelKind,
    AsyncChannelOp, AsyncChannelOutcome, AsyncChannelSide,
};

/// Creates a `tokio::sync::watch`-compatible model channel with `init` as
/// the initial value.
///
/// Reports one `channel_created` boundary (carrying
/// [`AsyncChannelKind::Watch`]) when a hook is installed.
pub fn channel<T>(init: T) -> (Sender<T>, Receiver<T>) {
    let (inner_tx, inner_rx) = tokio::sync::watch::channel(init);
    let resource = next_async_lock_resource_id();
    if let Some(hook) = async_channel_hook() {
        hook.channel_created(resource, AsyncChannelKind::Watch);
    }
    (
        Sender {
            inner: inner_tx,
            resource,
        },
        Receiver {
            inner: inner_rx,
            resource,
        },
    )
}

/// `tokio::sync::watch::Sender<T>` compatible model sender for annotated
/// code.
pub struct Sender<T> {
    inner: tokio::sync::watch::Sender<T>,
    resource: u64,
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        let cloned = self.inner.clone();
        if let Some(hook) = async_channel_hook() {
            hook.endpoint_cloned(self.resource, AsyncChannelSide::Sender);
        }
        Self {
            inner: cloned,
            resource: self.resource,
        }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        if let Some(hook) = async_channel_hook() {
            hook.endpoint_dropped(self.resource, AsyncChannelSide::Sender);
        }
    }
}

impl<T> Sender<T> {
    /// Sends a new value via the channel, notifying all receivers.
    ///
    /// Mirrors `tokio::sync::watch::Sender::send`. Reports one
    /// `op_resolved` boundary under a freshly allocated op id — a watch
    /// send never waits.
    ///
    /// # Errors
    ///
    /// Returns [`SendError`] if every [`Receiver`] has been dropped.
    pub fn send(&self, value: T) -> Result<(), SendError<T>> {
        let result = self.inner.send(value);
        let op = next_async_lock_waiter_id();
        if let Some(hook) = async_channel_hook() {
            let outcome = if result.is_ok() {
                AsyncChannelOutcome::Ok
            } else {
                AsyncChannelOutcome::Closed
            };
            hook.op_resolved(self.resource, op, AsyncChannelOp::Send, outcome);
        }
        result
    }

    /// Creates a new [`Receiver`] connected to this [`Sender`].
    ///
    /// Mirrors `tokio::sync::watch::Sender::subscribe`, reporting the new
    /// receiver via `endpoint_cloned` — from this seam's perspective a
    /// freshly subscribed receiver is the same endpoint-lifecycle event as
    /// an explicit `Receiver::clone`.
    #[must_use]
    pub fn subscribe(&self) -> Receiver<T> {
        let inner = self.inner.subscribe();
        if let Some(hook) = async_channel_hook() {
            hook.endpoint_cloned(self.resource, AsyncChannelSide::Receiver);
        }
        Receiver {
            inner,
            resource: self.resource,
        }
    }

    /// Returns whether every [`Receiver`] has been dropped.
    ///
    /// Mirrors `tokio::sync::watch::Sender::is_closed`. No hook boundary
    /// applies — this is a point-in-time read, not a send.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    /// Returns whether `self` and `other` are handles to the same channel.
    ///
    /// Mirrors `tokio::sync::watch::Sender::same_channel`. No hook boundary
    /// applies.
    #[must_use]
    pub fn same_channel(&self, other: &Self) -> bool {
        self.inner.same_channel(&other.inner)
    }
}

/// `tokio::sync::watch::Receiver<T>` compatible model receiver for
/// annotated code.
pub struct Receiver<T> {
    inner: tokio::sync::watch::Receiver<T>,
    resource: u64,
}

impl<T> Clone for Receiver<T> {
    fn clone(&self) -> Self {
        let cloned = self.inner.clone();
        if let Some(hook) = async_channel_hook() {
            hook.endpoint_cloned(self.resource, AsyncChannelSide::Receiver);
        }
        Self {
            inner: cloned,
            resource: self.resource,
        }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        if let Some(hook) = async_channel_hook() {
            hook.endpoint_dropped(self.resource, AsyncChannelSide::Receiver);
        }
    }
}

impl<T> Receiver<T> {
    /// Returns a reference to the most recently sent value.
    ///
    /// Mirrors `tokio::sync::watch::Receiver::borrow`. See the module's
    /// honesty contract for why this reports no hook boundary.
    #[must_use]
    pub fn borrow(&self) -> Ref<'_, T> {
        Ref {
            inner: self.inner.borrow(),
        }
    }

    /// Returns a reference to the most recently sent value, marking it as
    /// seen.
    ///
    /// Mirrors `tokio::sync::watch::Receiver::borrow_and_update`. See the
    /// module's honesty contract for why this reports no hook boundary.
    pub fn borrow_and_update(&mut self) -> Ref<'_, T> {
        Ref {
            inner: self.inner.borrow_and_update(),
        }
    }

    /// Checks whether this channel contains a value not yet seen by this
    /// receiver.
    ///
    /// Mirrors `tokio::sync::watch::Receiver::has_changed`. No hook
    /// boundary applies — this is a point-in-time read, not a wait.
    ///
    /// # Errors
    ///
    /// Returns [`RecvError`] if every [`Sender`] has been dropped.
    pub fn has_changed(&self) -> Result<bool, RecvError> {
        self.inner.has_changed()
    }

    /// Returns whether `self` and `other` are handles to the same channel.
    ///
    /// Mirrors `tokio::sync::watch::Receiver::same_channel`. No hook
    /// boundary applies.
    #[must_use]
    pub fn same_channel(&self, other: &Self) -> bool {
        self.inner.same_channel(&other.inner)
    }
}

impl<T: Send + Sync> Receiver<T> {
    /// Waits for a change notification, then marks the current value as
    /// seen.
    ///
    /// The signature mirrors `tokio::sync::watch::Receiver::changed`. When a
    /// hook is installed, a `requested` boundary is reported the first time
    /// this future's poll finds no pending change, and an `op_resolved`
    /// boundary is reported when it resolves (`Ok` on a new value, `Closed`
    /// once every `Sender` has dropped with nothing left to see).
    ///
    /// `T: Send + Sync` is required so the returned future stays `Send` like
    /// tokio's own `changed()` future — rewritten source spawned via
    /// `tokio::spawn` must keep compiling wherever the raw tokio equivalent
    /// compiled (Send-parity, asserted by the fidelity gate).
    pub fn changed(&mut self) -> ModelWatchChanged<'_> {
        ModelWatchChanged {
            resource: self.resource,
            op: next_async_lock_waiter_id(),
            inner: Box::pin(self.inner.changed()),
            requested_emitted: false,
            resolved: false,
        }
    }
}

/// Future returned by [`Receiver::changed`].
///
/// One instance identifies a single `changed()` call, not a task — mirrors
/// [`crate::ModelAsyncLock`] for why the future itself is the addressable
/// unit for the `op_requested`/`op_dropped` boundaries.
pub struct ModelWatchChanged<'a> {
    resource: u64,
    op: u64,
    inner: Pin<Box<dyn Future<Output = Result<(), RecvError>> + Send + 'a>>,
    requested_emitted: bool,
    resolved: bool,
}

impl Future for ModelWatchChanged<'_> {
    type Output = Result<(), RecvError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.inner.as_mut().poll(cx) {
            Poll::Pending => {
                if !self.requested_emitted {
                    self.requested_emitted = true;
                    if let Some(hook) = async_channel_hook() {
                        hook.op_requested(self.resource, self.op, AsyncChannelOp::Changed);
                    }
                }
                Poll::Pending
            }
            Poll::Ready(result) => {
                self.resolved = true;
                if let Some(hook) = async_channel_hook() {
                    let outcome = if result.is_ok() {
                        AsyncChannelOutcome::Ok
                    } else {
                        AsyncChannelOutcome::Closed
                    };
                    hook.op_resolved(self.resource, self.op, AsyncChannelOp::Changed, outcome);
                }
                Poll::Ready(result)
            }
        }
    }
}

impl Drop for ModelWatchChanged<'_> {
    fn drop(&mut self) {
        if self.requested_emitted && !self.resolved {
            if let Some(hook) = async_channel_hook() {
                hook.op_dropped(self.resource, self.op);
            }
        }
    }
}

/// Thin `Deref`-only wrapper around `tokio::sync::watch::Ref`, returned by
/// [`Receiver::borrow`] and [`Receiver::borrow_and_update`].
///
/// See the module's honesty contract for why borrowing reports no hook
/// boundary.
pub struct Ref<'a, T> {
    inner: tokio::sync::watch::Ref<'a, T>,
}

impl<T> Ref<'_, T> {
    /// Indicates whether the borrowed value is considered changed since it
    /// was last marked as seen.
    ///
    /// Mirrors `tokio::sync::watch::Ref::has_changed`.
    #[must_use]
    pub fn has_changed(&self) -> bool {
        self.inner.has_changed()
    }
}

impl<T> Deref for Ref<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
