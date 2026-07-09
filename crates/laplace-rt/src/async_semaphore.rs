// SPDX-License-Identifier: Apache-2.0
//! `tokio::sync::Semaphore`-compatible model async semaphore.
//!
//! ## Honesty contract
//!
//! - **Wrap-real, not reimplemented.** [`ModelAsyncSemaphore`] holds a real
//!   `tokio::sync::Semaphore` and delegates every acquisition to it; the
//!   semantics observed here — FIFO fairness (including a queued
//!   `acquire_many` blocking a later, individually satisfiable `acquire`),
//!   no barging, cancellation — are tokio's own, not a model reconstruction.
//! - **Differential fidelity gate**: `tests/async_semaphore_fidelity.rs`
//!   runs identical scenarios against raw `tokio::sync::Semaphore` and
//!   against this wrapper and asserts observationally equivalent outcomes
//!   (mirrors the Mutex/RwLock slices' gates).
//! - **`semaphore_created` is synthetic.** Unlike the other three async
//!   primitives, a semaphore has capacity at construction that no single
//!   `requested`/`acquired` boundary carries; this wrapper reports it once,
//!   lazily, immediately before this instance's first hook-observed
//!   boundary (see [`crate::AsyncLockHook::semaphore_created`]), not at
//!   `new`/`const_new` time — a `const_new` static has no opportunity to run
//!   hook-reporting code at construction.
//! - **`forget` reports no `released` boundary, by design.** A forgotten
//!   permit never returns to the semaphore; from the hook's perspective this
//!   waiter holds its permits forever, and reporting a `released` boundary
//!   for permits that will never actually be released would be a lie.
//! - **Loud residual (AXM2 A2-3 slice 2 scope cut).** `close`, `is_closed`,
//!   the owned (`Arc`-based) `*_owned` acquire family, and
//!   `forget_permits` are not provided by this wrapper — calling them on
//!   model code fails loudly at compile time (no such method), not silently
//!   at runtime.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use crate::hooks::{
    async_lock_hook, next_async_lock_waiter_id, AsyncAcquireKind, AsyncLockHook, AsyncResourceId,
};

/// `tokio::sync::Semaphore` compatible model async semaphore for annotated
/// code.
pub struct ModelAsyncSemaphore {
    resource: AsyncResourceId,
    inner: tokio::sync::Semaphore,
    initial_permits: usize,
    created_reported: AtomicBool,
}

/// Reports `semaphore_created` exactly once (lazily, at the first observed
/// boundary while a hook is installed) before returning the hook to the
/// caller for that boundary's own event. See the module's honesty contract.
fn hook_with_created(
    created_reported: &AtomicBool,
    resource: u64,
    initial_permits: usize,
) -> Option<Arc<dyn AsyncLockHook>> {
    let hook = async_lock_hook()?;
    if !created_reported.swap(true, Ordering::SeqCst) {
        hook.semaphore_created(resource, initial_permits);
    }
    Some(hook)
}

impl ModelAsyncSemaphore {
    /// Creates a new model async semaphore with `permits` initial capacity
    /// and a distinct process-local resource id, allocated immediately.
    #[must_use]
    pub fn new(permits: usize) -> Self {
        Self {
            resource: AsyncResourceId::new_eager(),
            inner: tokio::sync::Semaphore::new(permits),
            initial_permits: permits,
            created_reported: AtomicBool::new(false),
        }
    }

    /// Creates a new model async semaphore in a `const` context.
    ///
    /// Mirrors `tokio::sync::Semaphore::const_new`. The resource id is not
    /// allocated until this semaphore's first observed hook boundary.
    #[must_use]
    pub const fn const_new(permits: usize) -> Self {
        Self {
            resource: AsyncResourceId::new_lazy(),
            inner: tokio::sync::Semaphore::const_new(permits),
            initial_permits: permits,
            created_reported: AtomicBool::new(false),
        }
    }

    /// Returns the current number of available permits.
    ///
    /// Mirrors `tokio::sync::Semaphore::available_permits`. No hook boundary
    /// applies — this is a point-in-time read, not an acquisition.
    pub fn available_permits(&self) -> usize {
        self.inner.available_permits()
    }

    /// Increases the number of permits available.
    ///
    /// Mirrors `tokio::sync::Semaphore::add_permits`, reporting a
    /// `permits_added` boundary after forwarding to the real semaphore.
    pub fn add_permits(&self, n: usize) {
        self.inner.add_permits(n);
        let resource = self.resource.get();
        if let Some(hook) =
            hook_with_created(&self.created_reported, resource, self.initial_permits)
        {
            hook.permits_added(resource, n);
        }
    }

    /// Acquires one permit, returning a future that resolves to a permit
    /// guard.
    ///
    /// The signature mirrors `tokio::sync::Semaphore::acquire`. When a hook
    /// is installed, a `requested` boundary is reported the first time this
    /// future's poll finds contention, an `acquired` boundary is reported
    /// when the permit is produced, and a `released` boundary is reported
    /// when the returned permit is dropped (unless [`ModelSemaphorePermit::forget`]
    /// is called instead).
    pub fn acquire(&self) -> ModelSemaphoreAcquire<'_> {
        ModelSemaphoreAcquire {
            resource: self.resource.get(),
            waiter: next_async_lock_waiter_id(),
            permits: 1,
            inner: Box::pin(self.inner.acquire()),
            requested_emitted: false,
            resolved: false,
            created_reported: &self.created_reported,
            initial_permits: self.initial_permits,
        }
    }

    /// Acquires `n` permits, returning a future that resolves to a permit
    /// guard covering all `n`.
    ///
    /// Mirrors `tokio::sync::Semaphore::acquire_many`; boundary reporting is
    /// the same as [`ModelAsyncSemaphore::acquire`], with
    /// [`crate::AsyncAcquireKind::SemaphorePermits`] carrying `n`.
    pub fn acquire_many(&self, n: u32) -> ModelSemaphoreAcquire<'_> {
        ModelSemaphoreAcquire {
            resource: self.resource.get(),
            waiter: next_async_lock_waiter_id(),
            permits: n,
            inner: Box::pin(self.inner.acquire_many(n)),
            requested_emitted: false,
            resolved: false,
            created_reported: &self.created_reported,
            initial_permits: self.initial_permits,
        }
    }

    /// Attempts to acquire one permit without waiting.
    ///
    /// Mirrors `tokio::sync::Semaphore::try_acquire`. A successful
    /// acquisition reports one `acquired` boundary under a freshly
    /// allocated waiter id; a [`tokio::sync::TryAcquireError`] failure
    /// reports nothing.
    ///
    /// # Errors
    ///
    /// Returns [`tokio::sync::TryAcquireError`] if no permit is available.
    pub fn try_acquire(&self) -> Result<ModelSemaphorePermit<'_>, tokio::sync::TryAcquireError> {
        let inner = self.inner.try_acquire()?;
        let resource = self.resource.get();
        let waiter = next_async_lock_waiter_id();
        if let Some(hook) =
            hook_with_created(&self.created_reported, resource, self.initial_permits)
        {
            hook.acquired(resource, waiter, AsyncAcquireKind::SemaphorePermits(1));
        }
        Ok(ModelSemaphorePermit {
            inner: Some(inner),
            resource,
            waiter,
            permits: 1,
        })
    }

    /// Attempts to acquire `n` permits without waiting.
    ///
    /// Mirrors `tokio::sync::Semaphore::try_acquire_many`. Boundary
    /// reporting mirrors [`ModelAsyncSemaphore::try_acquire`].
    ///
    /// # Errors
    ///
    /// Returns [`tokio::sync::TryAcquireError`] if fewer than `n` permits
    /// are available.
    pub fn try_acquire_many(
        &self,
        n: u32,
    ) -> Result<ModelSemaphorePermit<'_>, tokio::sync::TryAcquireError> {
        let inner = self.inner.try_acquire_many(n)?;
        let resource = self.resource.get();
        let waiter = next_async_lock_waiter_id();
        if let Some(hook) =
            hook_with_created(&self.created_reported, resource, self.initial_permits)
        {
            hook.acquired(resource, waiter, AsyncAcquireKind::SemaphorePermits(n));
        }
        Ok(ModelSemaphorePermit {
            inner: Some(inner),
            resource,
            waiter,
            permits: n,
        })
    }
}

/// Future returned by [`ModelAsyncSemaphore::acquire`] and
/// [`ModelAsyncSemaphore::acquire_many`].
///
/// One instance identifies a single `acquire()`/`acquire_many()` call, not a
/// task — see [`crate::ModelAsyncLock`] for why the future itself is the
/// addressable unit for the `requested`/`waiter_dropped` boundaries.
pub struct ModelSemaphoreAcquire<'a> {
    resource: u64,
    waiter: u64,
    permits: u32,
    #[allow(clippy::type_complexity)]
    inner: Pin<
        Box<
            dyn Future<Output = Result<tokio::sync::SemaphorePermit<'a>, tokio::sync::AcquireError>>
                + Send
                + 'a,
        >,
    >,
    requested_emitted: bool,
    resolved: bool,
    created_reported: &'a AtomicBool,
    initial_permits: usize,
}

impl<'a> Future for ModelSemaphoreAcquire<'a> {
    type Output = Result<ModelSemaphorePermit<'a>, tokio::sync::AcquireError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.inner.as_mut().poll(cx) {
            Poll::Pending => {
                if !self.requested_emitted {
                    self.requested_emitted = true;
                    if let Some(hook) = hook_with_created(
                        self.created_reported,
                        self.resource,
                        self.initial_permits,
                    ) {
                        hook.requested(
                            self.resource,
                            self.waiter,
                            AsyncAcquireKind::SemaphorePermits(self.permits),
                        );
                    }
                }
                Poll::Pending
            }
            Poll::Ready(Ok(inner)) => {
                self.resolved = true;
                if let Some(hook) =
                    hook_with_created(self.created_reported, self.resource, self.initial_permits)
                {
                    hook.acquired(
                        self.resource,
                        self.waiter,
                        AsyncAcquireKind::SemaphorePermits(self.permits),
                    );
                }
                Poll::Ready(Ok(ModelSemaphorePermit {
                    inner: Some(inner),
                    resource: self.resource,
                    waiter: self.waiter,
                    permits: self.permits,
                }))
            }
            // The semaphore is closed. We never expose `close()` on this
            // model surface, so this arm is unreachable in practice — but it
            // must still be handled honestly (no panic, no fabricated
            // event) rather than assumed away.
            Poll::Ready(Err(err)) => {
                self.resolved = true;
                Poll::Ready(Err(err))
            }
        }
    }
}

impl Drop for ModelSemaphoreAcquire<'_> {
    fn drop(&mut self) {
        if self.requested_emitted && !self.resolved {
            if let Some(hook) = async_lock_hook() {
                hook.waiter_dropped(self.resource, self.waiter);
            }
        }
    }
}

/// Permit guard returned by a resolved [`ModelSemaphoreAcquire`] or by
/// [`ModelAsyncSemaphore::try_acquire`]/[`ModelAsyncSemaphore::try_acquire_many`].
#[derive(Debug)]
pub struct ModelSemaphorePermit<'a> {
    inner: Option<tokio::sync::SemaphorePermit<'a>>,
    resource: u64,
    waiter: u64,
    permits: u32,
}

impl ModelSemaphorePermit<'_> {
    /// Forgets the permit without releasing it back to the semaphore.
    ///
    /// Mirrors `tokio::sync::SemaphorePermit::forget`. See the module's
    /// honesty contract for why this emits no `released` boundary.
    pub fn forget(mut self) {
        if let Some(inner) = self.inner.take() {
            inner.forget();
        }
    }
}

impl Drop for ModelSemaphorePermit<'_> {
    fn drop(&mut self) {
        if self.inner.is_some() {
            if let Some(hook) = async_lock_hook() {
                hook.released(
                    self.resource,
                    self.waiter,
                    AsyncAcquireKind::SemaphorePermits(self.permits),
                );
            }
        }
    }
}
