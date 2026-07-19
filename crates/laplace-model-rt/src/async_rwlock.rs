// SPDX-License-Identifier: Apache-2.0
//! `tokio::sync::RwLock`-compatible model async rwlock.
//!
//! ## Honesty contract
//!
//! - **Wrap-real, not reimplemented.** [`ModelAsyncRwLock`] holds a real
//!   `tokio::sync::RwLock<T>` and delegates every acquisition to it; the
//!   semantics observed here — concurrent shared readers, FIFO fairness that
//!   also blocks a fresh reader behind a queued writer (write-starvation
//!   avoidance), no barging, cancellation — are tokio's own, not a model
//!   reconstruction.
//! - **Differential fidelity gate**: `tests/async_rwlock_fidelity.rs` runs
//!   identical scenarios against raw `tokio::sync::RwLock` and against this
//!   wrapper and asserts observationally equivalent outcomes (AXM2 decision
//!   doc §5.2, mirrors the Mutex slice's gate in
//!   `tests/async_mutex_fidelity.rs`).
//! - **No `Reserved` state in the event vocabulary.** Same reasoning as
//!   [`crate::ModelAsyncMutex`]: the internal permit handoff moment is not
//!   observable here, only the next poll of the reserved waiter.
//! - **Loud residual (AXM2 A2-3 slice 2 scope cut).** `blocking_read`,
//!   `blocking_write`, the owned (`Arc`-based) `*_owned` family,
//!   `downgrade`, and mapped guards are not provided by this wrapper —
//!   calling them on model code fails loudly at compile time (no such
//!   method), not silently at runtime.

use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::hooks::{async_lock_hook, next_async_lock_waiter_id, AsyncAcquireKind, AsyncResourceId};

/// `tokio::sync::RwLock<T>` compatible model async rwlock for annotated code.
pub struct ModelAsyncRwLock<T: ?Sized> {
    resource: AsyncResourceId,
    inner: tokio::sync::RwLock<T>,
}

impl<T> ModelAsyncRwLock<T> {
    /// Creates a new model async rwlock with a distinct process-local
    /// resource id, allocated immediately.
    pub fn new(t: T) -> Self {
        Self {
            resource: AsyncResourceId::new_eager(),
            inner: tokio::sync::RwLock::new(t),
        }
    }

    /// Creates a new model async rwlock in a `const` context.
    ///
    /// Mirrors `tokio::sync::RwLock::const_new`. The resource id is not
    /// allocated until this rwlock's first observed hook boundary.
    pub const fn const_new(t: T) -> Self {
        Self {
            resource: AsyncResourceId::new_lazy(),
            inner: tokio::sync::RwLock::const_new(t),
        }
    }
}

impl<T: ?Sized> ModelAsyncRwLock<T> {
    /// Consumes the rwlock, returning the underlying value.
    ///
    /// Mirrors `tokio::sync::RwLock::into_inner`. No hook boundary applies —
    /// there is no contention to observe once the rwlock is consumed.
    pub fn into_inner(self) -> T
    where
        T: Sized,
    {
        self.inner.into_inner()
    }

    /// Returns a mutable reference to the underlying data.
    ///
    /// Mirrors `tokio::sync::RwLock::get_mut`. No hook boundary applies — the
    /// borrow checker already guarantees exclusive access here, so there is
    /// no contention to observe.
    pub fn get_mut(&mut self) -> &mut T {
        self.inner.get_mut()
    }

    /// Attempts to acquire the rwlock with shared read access, without
    /// waiting.
    ///
    /// Mirrors `tokio::sync::RwLock::try_read`. A successful acquisition
    /// reports one `acquired` boundary under a freshly allocated waiter id;
    /// a [`tokio::sync::TryLockError`] failure reports nothing.
    ///
    /// # Errors
    ///
    /// Returns [`tokio::sync::TryLockError`] if a writer holds or is queued.
    pub fn try_read(&self) -> Result<ModelAsyncRwLockReadGuard<'_, T>, tokio::sync::TryLockError> {
        let inner = self.inner.try_read()?;
        let resource = self.resource.get();
        let waiter = next_async_lock_waiter_id();
        if let Some(hook) = async_lock_hook() {
            hook.acquired(resource, waiter, AsyncAcquireKind::RwRead);
        }
        Ok(ModelAsyncRwLockReadGuard {
            inner,
            resource,
            waiter,
        })
    }

    /// Attempts to acquire the rwlock with exclusive write access, without
    /// waiting.
    ///
    /// Mirrors `tokio::sync::RwLock::try_write`. A successful acquisition
    /// reports one `acquired` boundary under a freshly allocated waiter id;
    /// a [`tokio::sync::TryLockError`] failure reports nothing.
    ///
    /// # Errors
    ///
    /// Returns [`tokio::sync::TryLockError`] if any reader or writer holds
    /// or is queued.
    pub fn try_write(
        &self,
    ) -> Result<ModelAsyncRwLockWriteGuard<'_, T>, tokio::sync::TryLockError> {
        let inner = self.inner.try_write()?;
        let resource = self.resource.get();
        let waiter = next_async_lock_waiter_id();
        if let Some(hook) = async_lock_hook() {
            hook.acquired(resource, waiter, AsyncAcquireKind::RwWrite);
        }
        Ok(ModelAsyncRwLockWriteGuard {
            inner,
            resource,
            waiter,
        })
    }
}

impl<T: ?Sized + Send + Sync> ModelAsyncRwLock<T> {
    /// Acquires the rwlock with shared read access, returning a future that
    /// resolves to a guard.
    ///
    /// The signature mirrors `tokio::sync::RwLock::read`. When a hook is
    /// installed, a `requested` boundary is reported the first time this
    /// future's poll finds contention (a queued writer ahead of it, or a
    /// held writer), an `acquired` boundary is reported when the guard is
    /// produced, and a `released` boundary is reported when the returned
    /// guard is dropped.
    ///
    /// `T: Send + Sync` is required for the same reason as tokio's own
    /// `RwLock<T>: Sync` bound (see the module's honesty contract): the
    /// boxed future captures `&ModelAsyncRwLock<T>` across `.await`, so it
    /// is `Send` only when the rwlock itself is `Sync`.
    pub fn read(&self) -> ModelAsyncRead<'_, T> {
        ModelAsyncRead {
            resource: self.resource.get(),
            waiter: next_async_lock_waiter_id(),
            inner: Box::pin(self.inner.read()),
            requested_emitted: false,
            acquired: false,
        }
    }

    /// Acquires the rwlock with exclusive write access, returning a future
    /// that resolves to a guard.
    ///
    /// The signature mirrors `tokio::sync::RwLock::write`. Boundary
    /// reporting mirrors [`ModelAsyncRwLock::read`], with
    /// [`crate::AsyncAcquireKind::RwWrite`] instead.
    pub fn write(&self) -> ModelAsyncWrite<'_, T> {
        ModelAsyncWrite {
            resource: self.resource.get(),
            waiter: next_async_lock_waiter_id(),
            inner: Box::pin(self.inner.write()),
            requested_emitted: false,
            acquired: false,
        }
    }
}

/// Future returned by [`ModelAsyncRwLock::read`].
///
/// One instance identifies a single `read()` call, not a task — see
/// [`crate::ModelAsyncLock`] for why the future itself is the addressable
/// unit for the `requested`/`waiter_dropped` boundaries.
pub struct ModelAsyncRead<'a, T: ?Sized> {
    resource: u64,
    waiter: u64,
    inner: Pin<Box<dyn Future<Output = tokio::sync::RwLockReadGuard<'a, T>> + Send + 'a>>,
    requested_emitted: bool,
    acquired: bool,
}

impl<'a, T: ?Sized> Future for ModelAsyncRead<'a, T> {
    type Output = ModelAsyncRwLockReadGuard<'a, T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.inner.as_mut().poll(cx) {
            Poll::Pending => {
                if !self.requested_emitted {
                    self.requested_emitted = true;
                    if let Some(hook) = async_lock_hook() {
                        hook.requested(self.resource, self.waiter, AsyncAcquireKind::RwRead);
                    }
                }
                Poll::Pending
            }
            Poll::Ready(inner) => {
                self.acquired = true;
                if let Some(hook) = async_lock_hook() {
                    hook.acquired(self.resource, self.waiter, AsyncAcquireKind::RwRead);
                }
                Poll::Ready(ModelAsyncRwLockReadGuard {
                    inner,
                    resource: self.resource,
                    waiter: self.waiter,
                })
            }
        }
    }
}

impl<T: ?Sized> Drop for ModelAsyncRead<'_, T> {
    fn drop(&mut self) {
        if self.requested_emitted && !self.acquired {
            if let Some(hook) = async_lock_hook() {
                hook.waiter_dropped(self.resource, self.waiter);
            }
        }
    }
}

/// Future returned by [`ModelAsyncRwLock::write`]. See [`ModelAsyncRead`].
pub struct ModelAsyncWrite<'a, T: ?Sized> {
    resource: u64,
    waiter: u64,
    inner: Pin<Box<dyn Future<Output = tokio::sync::RwLockWriteGuard<'a, T>> + Send + 'a>>,
    requested_emitted: bool,
    acquired: bool,
}

impl<'a, T: ?Sized> Future for ModelAsyncWrite<'a, T> {
    type Output = ModelAsyncRwLockWriteGuard<'a, T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.inner.as_mut().poll(cx) {
            Poll::Pending => {
                if !self.requested_emitted {
                    self.requested_emitted = true;
                    if let Some(hook) = async_lock_hook() {
                        hook.requested(self.resource, self.waiter, AsyncAcquireKind::RwWrite);
                    }
                }
                Poll::Pending
            }
            Poll::Ready(inner) => {
                self.acquired = true;
                if let Some(hook) = async_lock_hook() {
                    hook.acquired(self.resource, self.waiter, AsyncAcquireKind::RwWrite);
                }
                Poll::Ready(ModelAsyncRwLockWriteGuard {
                    inner,
                    resource: self.resource,
                    waiter: self.waiter,
                })
            }
        }
    }
}

impl<T: ?Sized> Drop for ModelAsyncWrite<'_, T> {
    fn drop(&mut self) {
        if self.requested_emitted && !self.acquired {
            if let Some(hook) = async_lock_hook() {
                hook.waiter_dropped(self.resource, self.waiter);
            }
        }
    }
}

/// Guard returned by a resolved [`ModelAsyncRead`] or by
/// [`ModelAsyncRwLock::try_read`].
pub struct ModelAsyncRwLockReadGuard<'a, T: ?Sized> {
    inner: tokio::sync::RwLockReadGuard<'a, T>,
    resource: u64,
    waiter: u64,
}

impl<T: ?Sized> Deref for ModelAsyncRwLockReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T: ?Sized> Drop for ModelAsyncRwLockReadGuard<'_, T> {
    fn drop(&mut self) {
        if let Some(hook) = async_lock_hook() {
            hook.released(self.resource, self.waiter, AsyncAcquireKind::RwRead);
        }
    }
}

/// Guard returned by a resolved [`ModelAsyncWrite`] or by
/// [`ModelAsyncRwLock::try_write`].
pub struct ModelAsyncRwLockWriteGuard<'a, T: ?Sized> {
    inner: tokio::sync::RwLockWriteGuard<'a, T>,
    resource: u64,
    waiter: u64,
}

impl<T: ?Sized> Deref for ModelAsyncRwLockWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T: ?Sized> DerefMut for ModelAsyncRwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<T: ?Sized> Drop for ModelAsyncRwLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        if let Some(hook) = async_lock_hook() {
            hook.released(self.resource, self.waiter, AsyncAcquireKind::RwWrite);
        }
    }
}
