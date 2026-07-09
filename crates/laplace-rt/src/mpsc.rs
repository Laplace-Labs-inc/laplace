// SPDX-License-Identifier: Apache-2.0
//! `tokio::sync::mpsc`-compatible model channel (bounded and unbounded).
//!
//! ## Honesty contract
//!
//! - **Wrap-real, not reimplemented.** [`Sender`]/[`Receiver`] (and their
//!   unbounded counterparts) each hold a real `tokio::sync::mpsc` endpoint
//!   and delegate every send/receive to it; the semantics observed here —
//!   FIFO delivery, backpressure, cancellation — are tokio's own, not a
//!   model reconstruction.
//! - **Differential fidelity gate**: `tests/async_mpsc_fidelity.rs` runs
//!   identical scenarios against raw `tokio::sync::mpsc` and against this
//!   wrapper and asserts observationally equivalent outcomes (mirrors the
//!   Mutex/Semaphore slices' gates).
//! - **Errors are tokio's own types**, not a model reconstruction —
//!   [`tokio::sync::mpsc::error::SendError`],
//!   [`tokio::sync::mpsc::error::TrySendError`], and
//!   [`tokio::sync::mpsc::error::TryRecvError`] are returned unmodified, so
//!   `match` arms written against the real tokio error types keep compiling.
//! - **Loud residual (AXM2 A2-4 scope cut).** `reserve`/`reserve_many`/
//!   `reserve_owned` and their `Permit`/`PermitIterator`/`OwnedPermit`
//!   return types, `downgrade`/`WeakSender`, `blocking_recv`/
//!   `blocking_send`/`blocking_recv_many`, `recv_many`/`poll_recv_many`,
//!   `poll_recv`, `send_timeout`, and the `sender_strong_count`/
//!   `sender_weak_count`/`strong_count`/`weak_count` counters are not
//!   provided by this wrapper — calling them on model code fails loudly at
//!   compile time (no such method), not silently at runtime.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::sync::mpsc::error::{SendError, TryRecvError, TrySendError};

use crate::hooks::{
    async_channel_hook, next_async_lock_resource_id, next_async_lock_waiter_id, AsyncChannelKind,
    AsyncChannelOp, AsyncChannelOutcome, AsyncChannelSide,
};

/// Creates a bounded `tokio::sync::mpsc`-compatible model channel with
/// `buffer` capacity.
///
/// Reports one `channel_created` boundary (carrying `buffer` as
/// [`AsyncChannelKind::MpscBounded`]) when a hook is installed.
#[must_use]
pub fn channel<T>(buffer: usize) -> (Sender<T>, Receiver<T>) {
    let (inner_tx, inner_rx) = tokio::sync::mpsc::channel(buffer);
    let resource = next_async_lock_resource_id();
    if let Some(hook) = async_channel_hook() {
        hook.channel_created(resource, AsyncChannelKind::MpscBounded { capacity: buffer });
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

/// Creates an unbounded `tokio::sync::mpsc`-compatible model channel.
///
/// Reports one `channel_created` boundary (carrying
/// [`AsyncChannelKind::MpscUnbounded`]) when a hook is installed.
#[must_use]
pub fn unbounded_channel<T>() -> (UnboundedSender<T>, UnboundedReceiver<T>) {
    let (inner_tx, inner_rx) = tokio::sync::mpsc::unbounded_channel();
    let resource = next_async_lock_resource_id();
    if let Some(hook) = async_channel_hook() {
        hook.channel_created(resource, AsyncChannelKind::MpscUnbounded);
    }
    (
        UnboundedSender {
            inner: inner_tx,
            resource,
        },
        UnboundedReceiver {
            inner: inner_rx,
            resource,
        },
    )
}

/// `tokio::sync::mpsc::Sender<T>` compatible model sender for annotated
/// code.
pub struct Sender<T> {
    inner: tokio::sync::mpsc::Sender<T>,
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

impl<T: Send> Sender<T> {
    /// Sends a value, waiting until there is capacity.
    ///
    /// The signature mirrors `tokio::sync::mpsc::Sender::send`. When a hook
    /// is installed, a `requested` boundary is reported the first time this
    /// future's poll finds the channel full, and an `op_resolved` boundary
    /// is reported when the send completes (`Ok` once buffered, `Closed` if
    /// the receiver has gone away).
    ///
    /// `T: Send` is required so the returned future stays `Send` like
    /// tokio's own `send()` future — rewritten source spawned via
    /// `tokio::spawn` must keep compiling wherever the raw tokio equivalent
    /// compiled (Send-parity, asserted by the fidelity gate).
    pub fn send(&self, value: T) -> ModelMpscSend<'_, T> {
        ModelMpscSend {
            resource: self.resource,
            op: next_async_lock_waiter_id(),
            inner: Box::pin(self.inner.send(value)),
            requested_emitted: false,
            resolved: false,
        }
    }
}

impl<T> Sender<T> {
    /// Attempts to immediately send a value without waiting.
    ///
    /// Mirrors `tokio::sync::mpsc::Sender::try_send`. Reports one
    /// `op_resolved` boundary under a freshly allocated op id, honestly
    /// carrying `Full`/`Closed` on failure rather than staying silent.
    ///
    /// # Errors
    ///
    /// Returns [`TrySendError`] if the channel is full or the receiver has
    /// gone away.
    pub fn try_send(&self, message: T) -> Result<(), TrySendError<T>> {
        let result = self.inner.try_send(message);
        let op = next_async_lock_waiter_id();
        if let Some(hook) = async_channel_hook() {
            let outcome = match &result {
                Ok(()) => AsyncChannelOutcome::Ok,
                Err(TrySendError::Full(_)) => AsyncChannelOutcome::Full,
                Err(TrySendError::Closed(_)) => AsyncChannelOutcome::Closed,
            };
            hook.op_resolved(self.resource, op, AsyncChannelOp::Send, outcome);
        }
        result
    }

    /// Returns whether the receiver half has been dropped or explicitly
    /// closed.
    ///
    /// Mirrors `tokio::sync::mpsc::Sender::is_closed`. No hook boundary
    /// applies — this is a point-in-time read, not a send.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    /// Returns the current send capacity.
    ///
    /// Mirrors `tokio::sync::mpsc::Sender::capacity`. No hook boundary
    /// applies.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Returns the channel's total buffer capacity.
    ///
    /// Mirrors `tokio::sync::mpsc::Sender::max_capacity`. No hook boundary
    /// applies.
    #[must_use]
    pub fn max_capacity(&self) -> usize {
        self.inner.max_capacity()
    }

    /// Returns whether `self` and `other` are handles to the same channel.
    ///
    /// Mirrors `tokio::sync::mpsc::Sender::same_channel`. No hook boundary
    /// applies.
    #[must_use]
    pub fn same_channel(&self, other: &Self) -> bool {
        self.inner.same_channel(&other.inner)
    }
}

/// Future returned by [`Sender::send`].
///
/// One instance identifies a single `send()` call, not a task — mirrors
/// [`crate::ModelAsyncLock`] for why the future itself is the addressable
/// unit for the `op_requested`/`op_dropped` boundaries.
pub struct ModelMpscSend<'a, T> {
    resource: u64,
    op: u64,
    inner: Pin<Box<dyn Future<Output = Result<(), SendError<T>>> + Send + 'a>>,
    requested_emitted: bool,
    resolved: bool,
}

impl<T> Future for ModelMpscSend<'_, T> {
    type Output = Result<(), SendError<T>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.inner.as_mut().poll(cx) {
            Poll::Pending => {
                if !self.requested_emitted {
                    self.requested_emitted = true;
                    if let Some(hook) = async_channel_hook() {
                        hook.op_requested(self.resource, self.op, AsyncChannelOp::Send);
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
                    hook.op_resolved(self.resource, self.op, AsyncChannelOp::Send, outcome);
                }
                Poll::Ready(result)
            }
        }
    }
}

impl<T> Drop for ModelMpscSend<'_, T> {
    fn drop(&mut self) {
        if self.requested_emitted && !self.resolved {
            if let Some(hook) = async_channel_hook() {
                hook.op_dropped(self.resource, self.op);
            }
        }
    }
}

/// `tokio::sync::mpsc::Receiver<T>` compatible model receiver for annotated
/// code.
pub struct Receiver<T> {
    inner: tokio::sync::mpsc::Receiver<T>,
    resource: u64,
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        if let Some(hook) = async_channel_hook() {
            hook.endpoint_dropped(self.resource, AsyncChannelSide::Receiver);
        }
    }
}

impl<T: Send> Receiver<T> {
    /// Receives the next value, waiting until one is available.
    ///
    /// The signature mirrors `tokio::sync::mpsc::Receiver::recv`. Boundary
    /// reporting mirrors [`Sender::send`]; resolves `Ok` with a value or
    /// `Closed` (`None`) once every `Sender` has dropped.
    pub fn recv(&mut self) -> ModelMpscRecv<'_, T> {
        ModelMpscRecv {
            resource: self.resource,
            op: next_async_lock_waiter_id(),
            inner: Box::pin(self.inner.recv()),
            requested_emitted: false,
            resolved: false,
        }
    }
}

impl<T> Receiver<T> {
    /// Attempts to immediately receive a value without waiting.
    ///
    /// Mirrors `tokio::sync::mpsc::Receiver::try_recv`. Reports one
    /// `op_resolved` boundary under a freshly allocated op id, honestly
    /// carrying `Empty`/`Closed` on failure.
    ///
    /// # Errors
    ///
    /// Returns [`TryRecvError`] if the channel is empty or every `Sender`
    /// has dropped.
    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        let result = self.inner.try_recv();
        let op = next_async_lock_waiter_id();
        if let Some(hook) = async_channel_hook() {
            let outcome = match &result {
                Ok(_) => AsyncChannelOutcome::Ok,
                Err(TryRecvError::Empty) => AsyncChannelOutcome::Empty,
                Err(TryRecvError::Disconnected) => AsyncChannelOutcome::Closed,
            };
            hook.op_resolved(self.resource, op, AsyncChannelOp::Recv, outcome);
        }
        result
    }

    /// Closes the receiving half, preventing further `send`s from
    /// succeeding while allowing already-buffered values to still be
    /// received.
    ///
    /// Mirrors `tokio::sync::mpsc::Receiver::close`, reporting a
    /// `channel_closed` boundary after forwarding to the real receiver.
    pub fn close(&mut self) {
        self.inner.close();
        if let Some(hook) = async_channel_hook() {
            hook.channel_closed(self.resource);
        }
    }

    /// Returns whether the channel is closed.
    ///
    /// Mirrors `tokio::sync::mpsc::Receiver::is_closed`. No hook boundary
    /// applies.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    /// Returns whether the channel currently has no buffered values.
    ///
    /// Mirrors `tokio::sync::mpsc::Receiver::is_empty`. No hook boundary
    /// applies.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns the number of currently buffered values.
    ///
    /// Mirrors `tokio::sync::mpsc::Receiver::len`. No hook boundary applies.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns the current send capacity.
    ///
    /// Mirrors `tokio::sync::mpsc::Receiver::capacity`. No hook boundary
    /// applies.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// Returns the channel's total buffer capacity.
    ///
    /// Mirrors `tokio::sync::mpsc::Receiver::max_capacity`. No hook boundary
    /// applies.
    #[must_use]
    pub fn max_capacity(&self) -> usize {
        self.inner.max_capacity()
    }
}

/// Future returned by [`Receiver::recv`] and [`UnboundedReceiver::recv`].
///
/// One instance identifies a single `recv()` call, not a task — mirrors
/// [`ModelMpscSend`] for why the future itself is the addressable unit for
/// the `op_requested`/`op_dropped` boundaries.
pub struct ModelMpscRecv<'a, T> {
    resource: u64,
    op: u64,
    inner: Pin<Box<dyn Future<Output = Option<T>> + Send + 'a>>,
    requested_emitted: bool,
    resolved: bool,
}

impl<T> Future for ModelMpscRecv<'_, T> {
    type Output = Option<T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.inner.as_mut().poll(cx) {
            Poll::Pending => {
                if !self.requested_emitted {
                    self.requested_emitted = true;
                    if let Some(hook) = async_channel_hook() {
                        hook.op_requested(self.resource, self.op, AsyncChannelOp::Recv);
                    }
                }
                Poll::Pending
            }
            Poll::Ready(value) => {
                self.resolved = true;
                if let Some(hook) = async_channel_hook() {
                    let outcome = if value.is_some() {
                        AsyncChannelOutcome::Ok
                    } else {
                        AsyncChannelOutcome::Closed
                    };
                    hook.op_resolved(self.resource, self.op, AsyncChannelOp::Recv, outcome);
                }
                Poll::Ready(value)
            }
        }
    }
}

impl<T> Drop for ModelMpscRecv<'_, T> {
    fn drop(&mut self) {
        if self.requested_emitted && !self.resolved {
            if let Some(hook) = async_channel_hook() {
                hook.op_dropped(self.resource, self.op);
            }
        }
    }
}

/// `tokio::sync::mpsc::UnboundedSender<T>` compatible model sender for
/// annotated code.
pub struct UnboundedSender<T> {
    inner: tokio::sync::mpsc::UnboundedSender<T>,
    resource: u64,
}

impl<T> Clone for UnboundedSender<T> {
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

impl<T> Drop for UnboundedSender<T> {
    fn drop(&mut self) {
        if let Some(hook) = async_channel_hook() {
            hook.endpoint_dropped(self.resource, AsyncChannelSide::Sender);
        }
    }
}

impl<T> UnboundedSender<T> {
    /// Sends a value immediately; an unbounded channel never blocks.
    ///
    /// Mirrors `tokio::sync::mpsc::UnboundedSender::send`. Reports one
    /// `op_resolved` boundary under a freshly allocated op id — there is no
    /// wait cycle to queue behind on an unbounded channel.
    ///
    /// # Errors
    ///
    /// Returns [`SendError`] if the receiver has gone away.
    pub fn send(&self, message: T) -> Result<(), SendError<T>> {
        let result = self.inner.send(message);
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

    /// Returns whether the receiver half has been dropped.
    ///
    /// Mirrors `tokio::sync::mpsc::UnboundedSender::is_closed`. No hook
    /// boundary applies.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    /// Returns whether `self` and `other` are handles to the same channel.
    ///
    /// Mirrors `tokio::sync::mpsc::UnboundedSender::same_channel`. No hook
    /// boundary applies.
    #[must_use]
    pub fn same_channel(&self, other: &Self) -> bool {
        self.inner.same_channel(&other.inner)
    }
}

/// `tokio::sync::mpsc::UnboundedReceiver<T>` compatible model receiver for
/// annotated code.
pub struct UnboundedReceiver<T> {
    inner: tokio::sync::mpsc::UnboundedReceiver<T>,
    resource: u64,
}

impl<T> Drop for UnboundedReceiver<T> {
    fn drop(&mut self) {
        if let Some(hook) = async_channel_hook() {
            hook.endpoint_dropped(self.resource, AsyncChannelSide::Receiver);
        }
    }
}

impl<T: Send> UnboundedReceiver<T> {
    /// Receives the next value, waiting until one is available.
    ///
    /// The signature mirrors `tokio::sync::mpsc::UnboundedReceiver::recv`.
    /// Boundary reporting mirrors [`Receiver::recv`].
    pub fn recv(&mut self) -> ModelMpscRecv<'_, T> {
        ModelMpscRecv {
            resource: self.resource,
            op: next_async_lock_waiter_id(),
            inner: Box::pin(self.inner.recv()),
            requested_emitted: false,
            resolved: false,
        }
    }
}

impl<T> UnboundedReceiver<T> {
    /// Attempts to immediately receive a value without waiting.
    ///
    /// Mirrors `tokio::sync::mpsc::UnboundedReceiver::try_recv`. Boundary
    /// reporting mirrors [`Receiver::try_recv`].
    ///
    /// # Errors
    ///
    /// Returns [`TryRecvError`] if the channel is empty or every `Sender`
    /// has dropped.
    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        let result = self.inner.try_recv();
        let op = next_async_lock_waiter_id();
        if let Some(hook) = async_channel_hook() {
            let outcome = match &result {
                Ok(_) => AsyncChannelOutcome::Ok,
                Err(TryRecvError::Empty) => AsyncChannelOutcome::Empty,
                Err(TryRecvError::Disconnected) => AsyncChannelOutcome::Closed,
            };
            hook.op_resolved(self.resource, op, AsyncChannelOp::Recv, outcome);
        }
        result
    }

    /// Closes the receiving half, preventing further `send`s from
    /// succeeding while allowing already-buffered values to still be
    /// received.
    ///
    /// Mirrors `tokio::sync::mpsc::UnboundedReceiver::close`, reporting a
    /// `channel_closed` boundary after forwarding to the real receiver.
    pub fn close(&mut self) {
        self.inner.close();
        if let Some(hook) = async_channel_hook() {
            hook.channel_closed(self.resource);
        }
    }

    /// Returns whether the channel is closed.
    ///
    /// Mirrors `tokio::sync::mpsc::UnboundedReceiver::is_closed`. No hook
    /// boundary applies.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    /// Returns whether the channel currently has no buffered values.
    ///
    /// Mirrors `tokio::sync::mpsc::UnboundedReceiver::is_empty`. No hook
    /// boundary applies.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns the number of currently buffered values.
    ///
    /// Mirrors `tokio::sync::mpsc::UnboundedReceiver::len`. No hook boundary
    /// applies.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }
}
