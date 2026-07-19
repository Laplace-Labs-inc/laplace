// SPDX-License-Identifier: Apache-2.0
//! `tokio::sync::Notify`-compatible model async notify.
//!
//! ## Honesty contract
//!
//! - **Wrap-real, not reimplemented.** [`ModelAsyncNotify`] holds a real
//!   `tokio::sync::Notify` and delegates every wait/wake to it; the
//!   semantics observed here — a single stored permit, FIFO wake order,
//!   `notify_waiters` only reaching futures that were already polled at
//!   least once — are tokio's own, not a model reconstruction.
//! - **Differential fidelity gate**: `tests/async_notify_fidelity.rs` runs
//!   identical scenarios against raw `tokio::sync::Notify` and against this
//!   wrapper and asserts observationally equivalent outcomes (mirrors the
//!   Mutex/RwLock/Semaphore slices' gates).
//! - **Distinct vocabulary from the lock family.** Unlike
//!   [`crate::ModelAsyncMutex`]/[`crate::ModelAsyncRwLock`]/
//!   [`crate::ModelAsyncSemaphore`], `Notify` is not an acquisition —
//!   there is no held resource to release. This module reports through
//!   [`crate::AsyncNotifyHook`] (`wait_requested`/`wait_resolved`/
//!   `notify_one`/`notify_waiters`/`waiter_dropped`), not
//!   [`crate::AsyncLockHook`].
//! - **`notified()` registration requires a poll.** A `notified()` future
//!   that has never been polled is not queued against the `Notify` at all
//!   (mirrors tokio's own cancel-safety contract); this wrapper only reports
//!   `wait_requested` once that first poll observes no stored permit and
//!   actually queues.
//! - **Loud residual (AXM2 A2-3 slice 2 scope cut).** `notify_last` and
//!   `notified_owned` are not provided by this wrapper — calling them on
//!   model code fails loudly at compile time (no such method), not silently
//!   at runtime.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::hooks::{async_notify_hook, next_async_lock_waiter_id, AsyncResourceId};

/// `tokio::sync::Notify` compatible model async notify for annotated code.
pub struct ModelAsyncNotify {
    resource: AsyncResourceId,
    inner: tokio::sync::Notify,
}

impl ModelAsyncNotify {
    /// Creates a new model async notify with a distinct process-local
    /// resource id, allocated immediately.
    #[must_use]
    pub fn new() -> Self {
        Self {
            resource: AsyncResourceId::new_eager(),
            inner: tokio::sync::Notify::new(),
        }
    }

    /// Creates a new model async notify in a `const` context.
    ///
    /// Mirrors `tokio::sync::Notify::const_new`. The resource id is not
    /// allocated until this notify's first observed hook boundary.
    #[must_use]
    pub const fn const_new() -> Self {
        Self {
            resource: AsyncResourceId::new_lazy(),
            inner: tokio::sync::Notify::const_new(),
        }
    }

    /// Waits for a notification, returning a future that resolves once one
    /// arrives (immediately, if a permit from an earlier `notify_one()` is
    /// already stored).
    ///
    /// The signature mirrors `tokio::sync::Notify::notified`. When a hook is
    /// installed, a `wait_requested` boundary is reported the first time
    /// this future's poll finds no stored permit and queues, and a
    /// `wait_resolved` boundary is reported when the future completes
    /// (immediately via a stored permit — no `wait_requested` in that case —
    /// or by a subsequent wake).
    pub fn notified(&self) -> ModelNotified<'_> {
        ModelNotified {
            resource: self.resource.get(),
            waiter: next_async_lock_waiter_id(),
            inner: Box::pin(self.inner.notified()),
            requested_emitted: false,
            resolved: false,
        }
    }

    /// Notifies one waiting task, storing a permit for the next
    /// `notified().await` if none is currently waiting.
    ///
    /// Mirrors `tokio::sync::Notify::notify_one`, reporting a `notify_one`
    /// boundary before forwarding to the real notify.
    pub fn notify_one(&self) {
        let resource = self.resource.get();
        if let Some(hook) = async_notify_hook() {
            hook.notify_one(resource);
        }
        self.inner.notify_one();
    }

    /// Notifies all currently-registered waiting tasks. No permit is stored
    /// for future `notified()` calls.
    ///
    /// Mirrors `tokio::sync::Notify::notify_waiters`, reporting a
    /// `notify_waiters` boundary before forwarding to the real notify.
    pub fn notify_waiters(&self) {
        let resource = self.resource.get();
        if let Some(hook) = async_notify_hook() {
            hook.notify_waiters(resource);
        }
        self.inner.notify_waiters();
    }
}

impl Default for ModelAsyncNotify {
    fn default() -> Self {
        Self::new()
    }
}

/// Future returned by [`ModelAsyncNotify::notified`].
///
/// One instance identifies a single `notified()` call, not a task — see
/// [`crate::ModelAsyncLock`] for why the future itself is the addressable
/// unit for the `wait_requested`/`waiter_dropped` boundaries.
pub struct ModelNotified<'a> {
    resource: u64,
    waiter: u64,
    inner: Pin<Box<dyn Future<Output = ()> + Send + 'a>>,
    requested_emitted: bool,
    resolved: bool,
}

impl Future for ModelNotified<'_> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.inner.as_mut().poll(cx) {
            Poll::Pending => {
                if !self.requested_emitted {
                    self.requested_emitted = true;
                    if let Some(hook) = async_notify_hook() {
                        hook.wait_requested(self.resource, self.waiter);
                    }
                }
                Poll::Pending
            }
            Poll::Ready(()) => {
                self.resolved = true;
                if let Some(hook) = async_notify_hook() {
                    hook.wait_resolved(self.resource, self.waiter);
                }
                Poll::Ready(())
            }
        }
    }
}

impl Drop for ModelNotified<'_> {
    fn drop(&mut self) {
        if self.requested_emitted && !self.resolved {
            if let Some(hook) = async_notify_hook() {
                hook.waiter_dropped(self.resource, self.waiter);
            }
        }
    }
}
