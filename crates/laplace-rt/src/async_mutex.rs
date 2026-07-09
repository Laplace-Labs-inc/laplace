// SPDX-License-Identifier: Apache-2.0
//! `tokio::sync::Mutex`-compatible model async mutex.
//!
//! ## Honesty contract
//!
//! - **Wrap-real, not reimplemented.** [`ModelAsyncMutex`] holds a real
//!   `tokio::sync::Mutex<T>` and delegates every acquisition to it; the
//!   semantics observed here — FIFO queueing, no barging, head-only
//!   handoff, cancellation — are tokio's own, not a model reconstruction.
//! - **Differential fidelity gate**: `tests/async_mutex_fidelity.rs` runs
//!   identical scenarios against raw `tokio::sync::Mutex` and against this
//!   wrapper and asserts observationally equivalent outcomes (AXM2 decision
//!   doc §5.2). That test is the evidence this module's claim is true, not
//!   this doc comment.
//! - **No `Reserved` state in the event vocabulary.** tokio hands a permit
//!   to the head waiter internally when the holder drops, before that
//!   waiter is polled again; this wrapper cannot observe that internal
//!   reservation moment (only the next poll of the reserved waiter, which
//!   then resolves as `Acquired`). Reasoning about the reservation window
//!   itself is private-engine scope, not this seam's.
//! - **No `blocking_lock`.** The async model surface intentionally has no
//!   blocking escape hatch — calling blocking primitives from async model
//!   code should fail loudly at compile time, not open a silent hole in the
//!   verified surface.

use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::hooks::{async_lock_hook, next_async_lock_resource_id, next_async_lock_waiter_id};

/// `tokio::sync::Mutex<T>` compatible model async mutex for annotated code.
pub struct ModelAsyncMutex<T: ?Sized> {
    resource: u64,
    inner: tokio::sync::Mutex<T>,
}

impl<T> ModelAsyncMutex<T> {
    /// Creates a new model async mutex with a distinct process-local
    /// resource id.
    pub fn new(t: T) -> Self {
        Self {
            resource: next_async_lock_resource_id(),
            inner: tokio::sync::Mutex::new(t),
        }
    }

    /// Consumes the mutex, returning the underlying value.
    ///
    /// Mirrors `tokio::sync::Mutex::into_inner`. No hook boundary applies —
    /// there is no contention to observe once the mutex is consumed.
    pub fn into_inner(self) -> T {
        self.inner.into_inner()
    }
}

impl<T: ?Sized> ModelAsyncMutex<T> {
    /// Acquires the mutex, returning a future that resolves to a guard.
    ///
    /// The signature mirrors `tokio::sync::Mutex::lock`, allowing annotated
    /// source to keep `.lock().await` unchanged. When a hook is installed,
    /// a `requested` boundary is reported the first time this future's poll
    /// finds contention, an `acquired` boundary is reported when the guard
    /// is produced (immediately or by resolving a queued wait), and a
    /// `released` boundary is reported when the returned guard is dropped.
    pub fn lock(&self) -> ModelAsyncLock<'_, T> {
        ModelAsyncLock {
            resource: self.resource,
            waiter: next_async_lock_waiter_id(),
            inner: Box::pin(self.inner.lock()),
            requested_emitted: false,
            acquired: false,
        }
    }

    /// Attempts to acquire the mutex without waiting.
    ///
    /// Mirrors `tokio::sync::Mutex::try_lock`. A successful acquisition
    /// reports one `acquired` boundary (under a freshly allocated waiter
    /// id, since no `lock()` future was ever queued); a
    /// [`tokio::sync::TryLockError`] failure reports nothing, since a
    /// non-blocking failure holds no resource and cannot participate in a
    /// wait cycle.
    ///
    /// # Errors
    ///
    /// Returns [`tokio::sync::TryLockError`] if the lock is already held.
    pub fn try_lock(&self) -> Result<ModelAsyncMutexGuard<'_, T>, tokio::sync::TryLockError> {
        let inner = self.inner.try_lock()?;
        if let Some(hook) = async_lock_hook() {
            hook.acquired(self.resource, next_async_lock_waiter_id());
        }
        Ok(ModelAsyncMutexGuard {
            inner: Some(inner),
            resource: self.resource,
        })
    }

    /// Returns a mutable reference to the underlying data.
    ///
    /// Mirrors `tokio::sync::Mutex::get_mut`. No hook boundary applies — the
    /// borrow checker already guarantees exclusive access here, so there is
    /// no contention to observe.
    pub fn get_mut(&mut self) -> &mut T {
        self.inner.get_mut()
    }
}

/// Future returned by [`ModelAsyncMutex::lock`].
///
/// One instance identifies a single `lock()` call, not a task — a task may
/// hold a live, unpolled `lock()` future (the futurelock shape) while other
/// waiters queue behind it, so the future itself is the addressable unit
/// for the `requested`/`waiter_dropped` boundaries.
pub struct ModelAsyncLock<'a, T: ?Sized> {
    resource: u64,
    waiter: u64,
    inner: Pin<Box<dyn Future<Output = tokio::sync::MutexGuard<'a, T>> + 'a>>,
    requested_emitted: bool,
    acquired: bool,
}

impl<'a, T: ?Sized> Future for ModelAsyncLock<'a, T> {
    type Output = ModelAsyncMutexGuard<'a, T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.inner.as_mut().poll(cx) {
            Poll::Pending => {
                if !self.requested_emitted {
                    self.requested_emitted = true;
                    if let Some(hook) = async_lock_hook() {
                        hook.requested(self.resource, self.waiter);
                    }
                }
                Poll::Pending
            }
            Poll::Ready(inner) => {
                self.acquired = true;
                if let Some(hook) = async_lock_hook() {
                    hook.acquired(self.resource, self.waiter);
                }
                Poll::Ready(ModelAsyncMutexGuard {
                    inner: Some(inner),
                    resource: self.resource,
                })
            }
        }
    }
}

impl<T: ?Sized> Drop for ModelAsyncLock<'_, T> {
    fn drop(&mut self) {
        if self.requested_emitted && !self.acquired {
            if let Some(hook) = async_lock_hook() {
                hook.waiter_dropped(self.resource, self.waiter);
            }
        }
    }
}

/// Guard returned by a resolved [`ModelAsyncLock`] or by
/// [`ModelAsyncMutex::try_lock`].
pub struct ModelAsyncMutexGuard<'a, T: ?Sized> {
    inner: Option<tokio::sync::MutexGuard<'a, T>>,
    resource: u64,
}

impl<T: ?Sized> Deref for ModelAsyncMutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner
            .as_deref()
            .expect("model async mutex guard is present")
    }
}

impl<T: ?Sized> DerefMut for ModelAsyncMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner
            .as_deref_mut()
            .expect("model async mutex guard is present")
    }
}

impl<T: ?Sized> Drop for ModelAsyncMutexGuard<'_, T> {
    fn drop(&mut self) {
        if self.inner.is_some() {
            if let Some(hook) = async_lock_hook() {
                hook.released(self.resource);
            }
        }
    }
}
