// SPDX-License-Identifier: Apache-2.0
//! `tokio::sync::broadcast`-compatible wrap-real model channel.
//!
//! Modeled as of BCAST G4 keep (LEP-0027): `#[laplace::model]` rewrites
//! `tokio::sync::broadcast` here (the former `TOKIO_CHANNEL` unmodeled
//! marker was removed), the engine consumes the events for wake attribution
//! and terminal wait evidence, and the CLI replays them. `Sender` and
//! `Receiver` hold real tokio endpoints and delegate value delivery, wake
//! registration, ring overwrite, subscription cursors, lag handling, and
//! cancellation to tokio. The wrapper adds observation events only.
//!
//! The `at_seq` field is evidence bookkeeping, not a reimplementation of the
//! queue. Tokio does not expose its sequence number, so a per-channel counter
//! advances after each successful real send. `Lagged { missed }` always carries
//! tokio's error payload without recalculation.
//!
//! ## Provided surface
//!
//! `channel(capacity)`, `Sender::{send, subscribe, receiver_count, len,
//! is_empty}`, `Receiver::{recv, try_recv, resubscribe, len, is_empty}`, and
//! `Sender` clone/drop plus `Receiver` drop observation are provided.
//!
//! ## Loud residual cuts
//!
//! The following tokio 1.52.3 surface is intentionally absent and therefore
//! fails at compile time when used: `WeakSender`, `Sender::{new, downgrade,
//! same_channel, strong_count, weak_count, closed}`, `Receiver::{same_channel,
//! sender_strong_count, sender_weak_count, is_closed, blocking_recv}`, and
//! all `WeakSender` methods. This is a scope cut, not a silent approximation.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::sync::broadcast::error::{RecvError, SendError, TryRecvError};

use crate::hooks::{
    async_broadcast_hook, next_async_lock_resource_id, next_async_lock_waiter_id, AsyncBroadcastOp,
    AsyncBroadcastOutcome, AsyncChannelSide,
};

/// Creates a real tokio broadcast channel with W observation bookkeeping.
#[must_use]
pub fn channel<T: Clone>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let (inner_sender, inner_receiver) = tokio::sync::broadcast::channel(capacity);
    let resource = next_async_lock_resource_id();
    let send_seq = Arc::new(AtomicU64::new(0));
    let receiver_id = next_async_lock_waiter_id();

    if let Some(hook) = async_broadcast_hook() {
        hook.broadcast_created(resource, capacity);
        hook.subscribed(resource, receiver_id, 0);
    }

    (
        Sender {
            inner: inner_sender,
            resource,
            send_seq: Arc::clone(&send_seq),
        },
        Receiver {
            inner: inner_receiver,
            resource,
            id: receiver_id,
            send_seq,
        },
    )
}

/// W sender wrapping a real tokio broadcast sender.
pub struct Sender<T> {
    inner: tokio::sync::broadcast::Sender<T>,
    resource: u64,
    /// Per-channel send position reported to `subscribed`. A position, not an
    /// id namespace, so the fail-closed id policy (ENGAUD F8) does not apply:
    /// nothing is named by it and it cannot alias two entities into one.
    send_seq: Arc<AtomicU64>,
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        if let Some(hook) = async_broadcast_hook() {
            hook.endpoint_cloned(self.resource, AsyncChannelSide::Sender, None);
        }
        Self {
            inner: self.inner.clone(),
            resource: self.resource,
            send_seq: Arc::clone(&self.send_seq),
        }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        if let Some(hook) = async_broadcast_hook() {
            hook.endpoint_dropped(self.resource, AsyncChannelSide::Sender, None);
        }
    }
}

impl<T: Clone> Sender<T> {
    /// Sends a value through the real tokio broadcast channel.
    ///
    /// # Errors
    ///
    /// Returns tokio's [`SendError`] when no receiver is live.
    pub fn send(&self, value: T) -> Result<usize, SendError<T>> {
        let op = next_async_lock_waiter_id();
        if let Some(hook) = async_broadcast_hook() {
            hook.op_requested(self.resource, op, None, AsyncBroadcastOp::Send);
        }
        let result = self.inner.send(value);
        let outcome = match &result {
            Ok(receivers) => {
                self.send_seq.fetch_add(1, Ordering::SeqCst);
                AsyncBroadcastOutcome::Ok {
                    receivers: *receivers,
                }
            }
            Err(_) => AsyncBroadcastOutcome::Closed,
        };
        if let Some(hook) = async_broadcast_hook() {
            hook.op_resolved(self.resource, op, None, AsyncBroadcastOp::Send, outcome);
        }
        result
    }

    /// Creates a receiver positioned at the current tail.
    #[must_use]
    pub fn subscribe(&self) -> Receiver<T> {
        let receiver = Receiver {
            inner: self.inner.subscribe(),
            resource: self.resource,
            id: next_async_lock_waiter_id(),
            send_seq: Arc::clone(&self.send_seq),
        };
        if let Some(hook) = async_broadcast_hook() {
            hook.endpoint_cloned(self.resource, AsyncChannelSide::Receiver, Some(receiver.id));
            hook.subscribed(
                self.resource,
                receiver.id,
                self.send_seq.load(Ordering::SeqCst),
            );
        }
        receiver
    }

    /// Returns the number of live receivers.
    #[must_use]
    pub fn receiver_count(&self) -> usize {
        self.inner.receiver_count()
    }

    /// Returns the number of values retained for at least one receiver.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns whether no values are retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

/// W receiver wrapping a real tokio broadcast receiver.
pub struct Receiver<T> {
    inner: tokio::sync::broadcast::Receiver<T>,
    resource: u64,
    id: u64,
    send_seq: Arc<AtomicU64>,
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        if let Some(hook) = async_broadcast_hook() {
            hook.endpoint_dropped(self.resource, AsyncChannelSide::Receiver, Some(self.id));
        }
    }
}

impl<T: Clone> Receiver<T> {
    /// Returns the number of values retained for this receiver.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns whether this receiver has no retained value.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Creates a receiver at the current tail via the real tokio operation.
    #[must_use]
    pub fn resubscribe(&self) -> Self {
        let op = next_async_lock_waiter_id();
        let receiver = Self {
            inner: self.inner.resubscribe(),
            resource: self.resource,
            id: next_async_lock_waiter_id(),
            send_seq: Arc::clone(&self.send_seq),
        };
        if let Some(hook) = async_broadcast_hook() {
            hook.endpoint_cloned(self.resource, AsyncChannelSide::Receiver, Some(receiver.id));
            hook.subscribed(
                self.resource,
                receiver.id,
                self.send_seq.load(Ordering::SeqCst),
            );
            hook.op_resolved(
                self.resource,
                op,
                Some(self.id),
                AsyncBroadcastOp::Resubscribe,
                AsyncBroadcastOutcome::Ok { receivers: 1 },
            );
        }
        receiver
    }

    /// Receives the next value through tokio's cancel-safe future.
    pub fn recv(&mut self) -> ModelBroadcastRecv<'_, T>
    where
        T: Send,
    {
        ModelBroadcastRecv {
            resource: self.resource,
            receiver_id: self.id,
            op: next_async_lock_waiter_id(),
            inner: Box::pin(self.inner.recv()),
            requested_emitted: false,
            resolved: false,
        }
    }

    /// Attempts to receive a value without awaiting.
    ///
    /// # Errors
    ///
    /// Returns tokio's [`TryRecvError`] for empty, closed, or lagged reads.
    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        let op = next_async_lock_waiter_id();
        let result = self.inner.try_recv();
        let outcome = match &result {
            Ok(_) => AsyncBroadcastOutcome::Ok { receivers: 1 },
            Err(TryRecvError::Empty) => AsyncBroadcastOutcome::Empty,
            Err(TryRecvError::Closed) => AsyncBroadcastOutcome::Closed,
            Err(TryRecvError::Lagged(missed)) => AsyncBroadcastOutcome::Lagged { missed: *missed },
        };
        if let Some(hook) = async_broadcast_hook() {
            hook.op_resolved(
                self.resource,
                op,
                Some(self.id),
                AsyncBroadcastOp::TryRecv,
                outcome,
            );
        }
        result
    }
}

/// Future returned by [`Receiver::recv`].
pub struct ModelBroadcastRecv<'a, T> {
    resource: u64,
    receiver_id: u64,
    op: u64,
    inner: Pin<Box<dyn Future<Output = Result<T, RecvError>> + Send + 'a>>,
    requested_emitted: bool,
    resolved: bool,
}

impl<T> Future for ModelBroadcastRecv<'_, T> {
    type Output = Result<T, RecvError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        match self.inner.as_mut().poll(context) {
            Poll::Pending => {
                if !self.requested_emitted {
                    self.requested_emitted = true;
                    if let Some(hook) = async_broadcast_hook() {
                        hook.op_requested(
                            self.resource,
                            self.op,
                            Some(self.receiver_id),
                            AsyncBroadcastOp::Recv,
                        );
                    }
                }
                Poll::Pending
            }
            Poll::Ready(result) => {
                self.resolved = true;
                let outcome = match &result {
                    Ok(_) => AsyncBroadcastOutcome::Ok { receivers: 1 },
                    Err(RecvError::Closed) => AsyncBroadcastOutcome::Closed,
                    Err(RecvError::Lagged(missed)) => {
                        AsyncBroadcastOutcome::Lagged { missed: *missed }
                    }
                };
                if let Some(hook) = async_broadcast_hook() {
                    hook.op_resolved(
                        self.resource,
                        self.op,
                        Some(self.receiver_id),
                        AsyncBroadcastOp::Recv,
                        outcome,
                    );
                }
                Poll::Ready(result)
            }
        }
    }
}

impl<T> Drop for ModelBroadcastRecv<'_, T> {
    fn drop(&mut self) {
        if self.requested_emitted && !self.resolved {
            if let Some(hook) = async_broadcast_hook() {
                hook.op_dropped(self.resource, self.op);
            }
        }
    }
}
