// SPDX-License-Identifier: Apache-2.0
//! `tokio::sync::oneshot`-compatible model channel.
//!
//! ## Honesty contract
//!
//! - **Wrap-real, not reimplemented.** [`Sender`]/[`Receiver`] each hold a
//!   real `tokio::sync::oneshot` endpoint and delegate every send/receive to
//!   it; the semantics observed here — single-value delivery, cancellation —
//!   are tokio's own, not a model reconstruction.
//! - **Differential fidelity gate**: `tests/async_oneshot_fidelity.rs` runs
//!   identical scenarios against raw `tokio::sync::oneshot` and against this
//!   wrapper and asserts observationally equivalent outcomes.
//! - **Errors are tokio's own types**, not a model reconstruction —
//!   [`tokio::sync::oneshot::error::RecvError`] and
//!   [`tokio::sync::oneshot::error::TryRecvError`] are returned unmodified.
//!   `Sender::send`'s error carries the unsent value back (`Result<(), T>`),
//!   matching tokio exactly.
//! - **The receive op id is allocated once, at [`channel`] time**, not per
//!   poll. Unlike the lock family (where each `.lock()` call creates a
//!   distinct queued acquisition) or `mpsc` (where each `.send()`/`.recv()`
//!   call creates a distinct op), a oneshot channel has exactly one possible
//!   receive: [`Receiver`] itself implements `Future` and is consumed by
//!   `.await`, so the op's identity coincides with the channel's.
//! - **`Sender::send` uses an `Option<inner>` pattern** (mirrors
//!   [`crate::ModelSemaphorePermit::forget`]) so that calling `send`, which
//!   consumes `self`, reports exactly one `op_resolved` boundary and never a
//!   redundant `endpoint_dropped` from the value's subsequent `Drop`.
//! - **Loud residual (AXM2 A2-4 scope cut).** `Sender::closed`/
//!   `Sender::poll_closed` and `Receiver::blocking_recv` are not provided by
//!   this wrapper — calling them on model code fails loudly at compile time
//!   (no such method), not silently at runtime.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::sync::oneshot::error::{RecvError, TryRecvError};

use crate::hooks::{
    async_channel_hook, next_async_lock_resource_id, next_async_lock_waiter_id, AsyncChannelKind,
    AsyncChannelOp, AsyncChannelOutcome, AsyncChannelSide,
};

/// Creates a `tokio::sync::oneshot`-compatible model channel.
///
/// Reports one `channel_created` boundary (carrying
/// [`AsyncChannelKind::Oneshot`]) when a hook is installed.
#[must_use]
pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let (inner_tx, inner_rx) = tokio::sync::oneshot::channel();
    let resource = next_async_lock_resource_id();
    if let Some(hook) = async_channel_hook() {
        hook.channel_created(resource, AsyncChannelKind::Oneshot);
    }
    let op = next_async_lock_waiter_id();
    (
        Sender {
            inner: Some(inner_tx),
            resource,
        },
        Receiver {
            inner: inner_rx,
            resource,
            op,
            requested_emitted: false,
            resolved: false,
        },
    )
}

/// `tokio::sync::oneshot::Sender<T>` compatible model sender for annotated
/// code.
pub struct Sender<T> {
    inner: Option<tokio::sync::oneshot::Sender<T>>,
    resource: u64,
}

impl<T> Sender<T> {
    /// Attempts to send a value on this channel, returning it back if it
    /// could not be sent.
    ///
    /// Mirrors `tokio::sync::oneshot::Sender::send`. Reports one
    /// `op_resolved` boundary under a freshly allocated op id — a oneshot
    /// send never waits.
    ///
    /// # Errors
    ///
    /// Returns `Err(t)` if the [`Receiver`] has already been dropped.
    ///
    /// # Panics
    ///
    /// Does not panic in practice: `inner` is only ever `None` transiently
    /// inside this method, and `send` consumes `self` by value, so no other
    /// call site can observe that state.
    pub fn send(mut self, t: T) -> Result<(), T> {
        // SAFETY: `inner` is only ever `None` transiently inside this
        // method, which consumes `self` by value — no other call site can
        // observe that state, so this is always `Some` on entry.
        let inner = self.inner.take().expect("oneshot Sender inner missing");
        let result = inner.send(t);
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

    /// Returns whether the associated [`Receiver`] has been dropped.
    ///
    /// Mirrors `tokio::sync::oneshot::Sender::is_closed`. No hook boundary
    /// applies — this is a point-in-time read, not a send.
    ///
    /// # Panics
    ///
    /// Does not panic in practice: `inner` is only ever `None` transiently
    /// inside [`Sender::send`], which consumes `self` — no live `&self` can
    /// observe that state.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        // SAFETY: `inner` is only ever `None` transiently inside `send`,
        // which consumes `self` — no live `&self` can observe that state.
        self.inner
            .as_ref()
            .expect("oneshot Sender inner missing")
            .is_closed()
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        if self.inner.is_some() {
            if let Some(hook) = async_channel_hook() {
                hook.endpoint_dropped(self.resource, AsyncChannelSide::Sender);
            }
        }
    }
}

/// `tokio::sync::oneshot::Receiver<T>` compatible model receiver for
/// annotated code.
///
/// Implements `Future` directly, mirroring
/// `tokio::sync::oneshot::Receiver`'s own `Future` impl — `rx.await` stays
/// source-compatible with the raw tokio type.
pub struct Receiver<T> {
    inner: tokio::sync::oneshot::Receiver<T>,
    resource: u64,
    op: u64,
    requested_emitted: bool,
    resolved: bool,
}

impl<T> Receiver<T> {
    /// Attempts to immediately receive a value without waiting.
    ///
    /// Mirrors `tokio::sync::oneshot::Receiver::try_recv`. Reports one
    /// `op_resolved` boundary under a freshly allocated op id, honestly
    /// carrying `Empty`/`Closed` on failure.
    ///
    /// # Errors
    ///
    /// Returns [`TryRecvError`] if no value has been sent yet, or the
    /// [`Sender`] was dropped without sending.
    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        let result = self.inner.try_recv();
        let op = next_async_lock_waiter_id();
        if let Some(hook) = async_channel_hook() {
            let outcome = match &result {
                Ok(_) => AsyncChannelOutcome::Ok,
                Err(TryRecvError::Empty) => AsyncChannelOutcome::Empty,
                Err(TryRecvError::Closed) => AsyncChannelOutcome::Closed,
            };
            hook.op_resolved(self.resource, op, AsyncChannelOp::Recv, outcome);
        }
        result
    }

    /// Prevents the associated [`Sender`] from sending a value.
    ///
    /// Mirrors `tokio::sync::oneshot::Receiver::close`, reporting a
    /// `channel_closed` boundary after forwarding to the real receiver.
    pub fn close(&mut self) {
        self.inner.close();
        if let Some(hook) = async_channel_hook() {
            hook.channel_closed(self.resource);
        }
    }
}

impl<T> Future for Receiver<T> {
    type Output = Result<T, RecvError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll(cx) {
            Poll::Pending => {
                if !this.requested_emitted {
                    this.requested_emitted = true;
                    if let Some(hook) = async_channel_hook() {
                        hook.op_requested(this.resource, this.op, AsyncChannelOp::Recv);
                    }
                }
                Poll::Pending
            }
            Poll::Ready(result) => {
                this.resolved = true;
                if let Some(hook) = async_channel_hook() {
                    let outcome = if result.is_ok() {
                        AsyncChannelOutcome::Ok
                    } else {
                        AsyncChannelOutcome::Closed
                    };
                    hook.op_resolved(this.resource, this.op, AsyncChannelOp::Recv, outcome);
                }
                Poll::Ready(result)
            }
        }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        if let Some(hook) = async_channel_hook() {
            if self.requested_emitted && !self.resolved {
                hook.op_dropped(self.resource, self.op);
            }
            hook.endpoint_dropped(self.resource, AsyncChannelSide::Receiver);
        }
    }
}
