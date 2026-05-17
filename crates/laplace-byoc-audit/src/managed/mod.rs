// SPDX-License-Identifier: Apache-2.0
//! Managed version of the pool — patched for Axiom DPOR verification.
//!
//! This is a verbatim copy of `deadpool 0.10.0 src/managed/mod.rs` with a
//! single **surgical patch** on the `tokio::sync` import (line 76 original).
//! All pool logic, invariants, and behaviour are unchanged.
//!
//! # Patch Location
//!
//! ```text
//! Original line 76:
//!   use tokio::sync::{Semaphore, TryAcquireError};
//!
//! Patched (feature = "axiom"):
//!   use crate::axiom_compat::{AxiomSemaphore as Semaphore, AxiomTryAcquireError as TryAcquireError};
//! ```
//!
//! See [`crate::axiom_compat`] for the `AxiomSemaphore` implementation.
#![allow(missing_docs)]
#![allow(dead_code)]
#![allow(unused_results)]

mod builder;
mod config;
mod dropguard;
mod errors;
mod hooks;
mod metrics;
pub mod reexports;

use std::{
    collections::VecDeque,
    fmt,
    future::Future,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, Weak,
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use deadpool_runtime::Runtime;

// ── SURGICAL PATCH: tokio::sync replaced by AxiomSync ──────────────────────
//
// Original deadpool 0.10.0 line 76:
//   use tokio::sync::{Semaphore, TryAcquireError};
//
// When `feature = "axiom"` is enabled the tokio Semaphore is replaced by
// `AxiomSemaphore`, which mirrors the tokio API but additionally records
// every acquire/release to the thread-local op_log for DPOR consumption.
// See `crate::axiom_compat` for implementation details.
//
// When `feature = "axiom"` is NOT enabled the original tokio semaphore is
// used unchanged — the pool behaves exactly as upstream deadpool.
#[cfg(feature = "axiom")]
use crate::axiom_compat::{AxiomSemaphore as Semaphore, AxiomTryAcquireError as TryAcquireError};
#[cfg(not(feature = "axiom"))]
use tokio::sync::{Semaphore, TryAcquireError};

pub use crate::Status;

use self::dropguard::DropGuard;
pub use self::{
    builder::{BuildError, PoolBuilder},
    config::{CreatePoolError, PoolConfig, QueueMode, Timeouts},
    errors::{PoolError, RecycleError, TimeoutType},
    hooks::{Hook, HookError, HookFuture, HookResult},
    metrics::Metrics,
};

/// Result type of the [`Manager::recycle()`] method.
pub type RecycleResult<E> = Result<(), RecycleError<E>>;

/// Manager responsible for creating new [`Object`]s or recycling existing ones.
#[async_trait]
pub trait Manager: Sync + Send {
    /// Type of [`Object`]s that this [`Manager`] creates and recycles.
    type Type;
    /// Error that this [`Manager`] can return when creating and/or recycling
    /// [`Object`]s.
    type Error;

    /// Creates a new instance of [`Manager::Type`].
    async fn create(&self) -> Result<Self::Type, Self::Error>;

    /// Tries to recycle an instance of [`Manager::Type`].
    async fn recycle(&self, obj: &mut Self::Type, metrics: &Metrics) -> RecycleResult<Self::Error>;

    /// Detaches an instance of [`Manager::Type`] from this [`Manager`].
    fn detach(&self, _obj: &mut Self::Type) {}
}

/// Wrapper around the actual pooled object which implements [`Deref`],
/// [`DerefMut`] and [`Drop`] traits.
#[must_use]
pub struct Object<M: Manager> {
    inner: Option<ObjectInner<M>>,
    pool: Weak<PoolInner<M>>,
}

impl<M> fmt::Debug for Object<M>
where
    M: fmt::Debug + Manager,
    M::Type: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Object")
            .field("inner", &self.inner)
            .finish()
    }
}

struct UnreadyObject<'a, M: Manager> {
    inner: Option<ObjectInner<M>>,
    pool: &'a PoolInner<M>,
}

impl<'a, M: Manager> UnreadyObject<'a, M> {
    fn ready(mut self) -> ObjectInner<M> {
        self.inner.take().unwrap()
    }
    fn inner(&mut self) -> &mut ObjectInner<M> {
        self.inner.as_mut().unwrap()
    }
}

impl<'a, M: Manager> Drop for UnreadyObject<'a, M> {
    fn drop(&mut self) {
        if let Some(mut inner) = self.inner.take() {
            self.pool.slots.lock().unwrap().size -= 1;
            self.pool.manager.detach(&mut inner.obj);
        }
    }
}

#[derive(Debug)]
pub(crate) struct ObjectInner<M: Manager> {
    obj: M::Type,
    metrics: Metrics,
}

impl<M: Manager> Object<M> {
    #[must_use]
    pub fn take(mut this: Self) -> M::Type {
        let mut inner = this.inner.take().unwrap().obj;
        if let Some(pool) = Object::pool(&this) {
            pool.inner.detach_object(&mut inner)
        }
        inner
    }

    pub fn metrics(this: &Self) -> &Metrics {
        &this.inner.as_ref().unwrap().metrics
    }

    pub fn pool(this: &Self) -> Option<Pool<M>> {
        this.pool.upgrade().map(|inner| Pool {
            inner,
            _wrapper: PhantomData,
        })
    }
}

impl<M: Manager> Drop for Object<M> {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            if let Some(pool) = self.pool.upgrade() {
                pool.return_object(inner)
            }
        }
    }
}

impl<M: Manager> Deref for Object<M> {
    type Target = M::Type;
    fn deref(&self) -> &M::Type {
        &self.inner.as_ref().unwrap().obj
    }
}

impl<M: Manager> DerefMut for Object<M> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner.as_mut().unwrap().obj
    }
}

impl<M: Manager> AsRef<M::Type> for Object<M> {
    fn as_ref(&self) -> &M::Type {
        self
    }
}

impl<M: Manager> AsMut<M::Type> for Object<M> {
    fn as_mut(&mut self) -> &mut M::Type {
        self
    }
}

/// Generic object and connection pool.
pub struct Pool<M: Manager, W: From<Object<M>> = Object<M>> {
    inner: Arc<PoolInner<M>>,
    _wrapper: PhantomData<fn() -> W>,
}

impl<M, W> fmt::Debug for Pool<M, W>
where
    M: fmt::Debug + Manager,
    M::Type: fmt::Debug,
    W: From<Object<M>>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pool")
            .field("inner", &self.inner)
            .field("wrapper", &self._wrapper)
            .finish()
    }
}

impl<M: Manager, W: From<Object<M>>> Clone for Pool<M, W> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _wrapper: PhantomData,
        }
    }
}

impl<M: Manager, W: From<Object<M>>> Pool<M, W> {
    pub fn builder(manager: M) -> PoolBuilder<M, W> {
        PoolBuilder::new(manager)
    }

    pub(crate) fn from_builder(builder: PoolBuilder<M, W>) -> Self {
        Self {
            inner: Arc::new(PoolInner {
                manager: builder.manager,
                slots: Mutex::new(Slots {
                    vec: VecDeque::with_capacity(builder.config.max_size),
                    size: 0,
                    max_size: builder.config.max_size,
                }),
                users: AtomicUsize::new(0),
                semaphore: Semaphore::new(builder.config.max_size),
                config: builder.config,
                hooks: builder.hooks,
                runtime: builder.runtime,
            }),
            _wrapper: PhantomData,
        }
    }

    pub async fn get(&self) -> Result<W, PoolError<M::Error>> {
        self.timeout_get(&self.timeouts()).await
    }

    pub async fn timeout_get(&self, timeouts: &Timeouts) -> Result<W, PoolError<M::Error>> {
        let _ = self.inner.users.fetch_add(1, Ordering::Relaxed);
        let users_guard = DropGuard(|| {
            let _ = self.inner.users.fetch_sub(1, Ordering::Relaxed);
        });

        let non_blocking = match timeouts.wait {
            Some(t) => t.as_nanos() == 0,
            None => false,
        };

        let permit = if non_blocking {
            self.inner.semaphore.try_acquire().map_err(|e| match e {
                TryAcquireError::Closed => PoolError::Closed,
                TryAcquireError::NoPermits => PoolError::Timeout(TimeoutType::Wait),
            })?
        } else {
            apply_timeout(
                self.inner.runtime,
                TimeoutType::Wait,
                timeouts.wait,
                async {
                    self.inner
                        .semaphore
                        .acquire()
                        .await
                        .map_err(|_| PoolError::Closed)
                },
            )
            .await?
        };

        let inner_obj = loop {
            let inner_obj = match self.inner.config.queue_mode {
                QueueMode::Fifo => self.inner.slots.lock().unwrap().vec.pop_front(),
                QueueMode::Lifo => self.inner.slots.lock().unwrap().vec.pop_back(),
            };
            let inner_obj = if let Some(inner_obj) = inner_obj {
                self.try_recycle(timeouts, inner_obj).await?
            } else {
                self.try_create(timeouts).await?
            };
            if let Some(inner_obj) = inner_obj {
                break inner_obj;
            }
        };

        users_guard.disarm();
        permit.forget();

        Ok(Object {
            inner: Some(inner_obj),
            pool: Arc::downgrade(&self.inner),
        }
        .into())
    }

    #[inline]
    async fn try_recycle(
        &self,
        timeouts: &Timeouts,
        inner_obj: ObjectInner<M>,
    ) -> Result<Option<ObjectInner<M>>, PoolError<M::Error>> {
        let mut unready_obj = UnreadyObject {
            inner: Some(inner_obj),
            pool: &self.inner,
        };
        let inner = unready_obj.inner();

        if let Err(_e) = self.inner.hooks.pre_recycle.apply(inner).await {
            return Ok(None);
        }

        if apply_timeout(
            self.inner.runtime,
            TimeoutType::Recycle,
            timeouts.recycle,
            self.inner.manager.recycle(&mut inner.obj, &inner.metrics),
        )
        .await
        .is_err()
        {
            return Ok(None);
        }

        if let Err(_e) = self.inner.hooks.post_recycle.apply(inner).await {
            return Ok(None);
        }

        inner.metrics.recycle_count += 1;
        inner.metrics.recycled = Some(Instant::now());

        Ok(Some(unready_obj.ready()))
    }

    #[inline]
    async fn try_create(
        &self,
        timeouts: &Timeouts,
    ) -> Result<Option<ObjectInner<M>>, PoolError<M::Error>> {
        let mut unready_obj = UnreadyObject {
            inner: Some(ObjectInner {
                obj: apply_timeout(
                    self.inner.runtime,
                    TimeoutType::Create,
                    timeouts.create,
                    self.inner.manager.create(),
                )
                .await?,
                metrics: Metrics::default(),
            }),
            pool: &self.inner,
        };

        self.inner.slots.lock().unwrap().size += 1;

        if let Err(e) = self
            .inner
            .hooks
            .post_create
            .apply(unready_obj.inner())
            .await
        {
            return Err(PoolError::PostCreateHook(e));
        }

        Ok(Some(unready_obj.ready()))
    }

    pub fn resize(&self, max_size: usize) {
        if self.inner.semaphore.is_closed() {
            return;
        }
        let mut slots = self.inner.slots.lock().unwrap();
        let old_max_size = slots.max_size;
        slots.max_size = max_size;
        if max_size < old_max_size {
            while slots.size > slots.max_size {
                if let Ok(permit) = self.inner.semaphore.try_acquire() {
                    permit.forget();
                    if slots.vec.pop_front().is_some() {
                        slots.size -= 1;
                    }
                } else {
                    break;
                }
            }
            let mut vec = VecDeque::with_capacity(max_size);
            for obj in slots.vec.drain(..) {
                vec.push_back(obj);
            }
            slots.vec = vec;
        }
        if max_size > old_max_size {
            let additional = slots.max_size - slots.size;
            slots.vec.reserve_exact(additional);
            self.inner.semaphore.add_permits(additional);
        }
    }

    pub fn retain(&self, f: impl Fn(&M::Type, Metrics) -> bool) {
        let mut guard = self.inner.slots.lock().unwrap();
        let len_before = guard.vec.len();
        guard.vec.retain_mut(|obj| {
            if f(&obj.obj, obj.metrics) {
                true
            } else {
                self.manager().detach(&mut obj.obj);
                false
            }
        });
        guard.size -= len_before - guard.vec.len();
    }

    pub fn timeouts(&self) -> Timeouts {
        self.inner.config.timeouts
    }

    pub fn close(&self) {
        self.resize(0);
        self.inner.semaphore.close();
    }

    pub fn is_closed(&self) -> bool {
        self.inner.semaphore.is_closed()
    }

    #[must_use]
    pub fn status(&self) -> crate::Status {
        let slots = self.inner.slots.lock().unwrap();
        let users = self.inner.users.load(Ordering::Relaxed);
        let (available, waiting) = if users < slots.size {
            (slots.size - users, 0)
        } else {
            (0, users - slots.size)
        };
        crate::Status {
            max_size: slots.max_size,
            size: slots.size,
            available,
            waiting,
        }
    }

    #[must_use]
    pub fn manager(&self) -> &M {
        &self.inner.manager
    }
}

struct PoolInner<M: Manager> {
    manager: M,
    slots: Mutex<Slots<ObjectInner<M>>>,
    users: AtomicUsize,
    semaphore: Semaphore,
    config: PoolConfig,
    runtime: Option<Runtime>,
    hooks: hooks::Hooks<M>,
}

#[derive(Debug)]
struct Slots<T> {
    vec: VecDeque<T>,
    size: usize,
    max_size: usize,
}

impl<M> fmt::Debug for PoolInner<M>
where
    M: fmt::Debug + Manager,
    M::Type: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PoolInner")
            .field("manager", &self.manager)
            .field("slots", &self.slots)
            .field("used", &self.users)
            .field("semaphore", &self.semaphore)
            .field("config", &self.config)
            .field("runtime", &self.runtime)
            .field("hooks", &self.hooks)
            .finish()
    }
}

impl<M: Manager> PoolInner<M> {
    fn return_object(&self, mut inner: ObjectInner<M>) {
        let _ = self.users.fetch_sub(1, Ordering::Relaxed);
        let mut slots = self.slots.lock().unwrap();
        if slots.size <= slots.max_size {
            slots.vec.push_back(inner);
            drop(slots);
            self.semaphore.add_permits(1);
        } else {
            slots.size -= 1;
            drop(slots);
            self.manager.detach(&mut inner.obj);
        }
    }
    fn detach_object(&self, obj: &mut M::Type) {
        let _ = self.users.fetch_sub(1, Ordering::Relaxed);
        let mut slots = self.slots.lock().unwrap();
        let add_permits = slots.size <= slots.max_size;
        slots.size -= 1;
        drop(slots);
        if add_permits {
            self.semaphore.add_permits(1);
        }
        self.manager.detach(obj);
    }
}

async fn apply_timeout<O, E>(
    runtime: Option<Runtime>,
    timeout_type: TimeoutType,
    duration: Option<Duration>,
    future: impl Future<Output = Result<O, impl Into<PoolError<E>>>>,
) -> Result<O, PoolError<E>> {
    match (runtime, duration) {
        (_, None) => future.await.map_err(Into::into),
        (Some(runtime), Some(duration)) => runtime
            .timeout(duration, future)
            .await
            .ok_or(PoolError::Timeout(timeout_type))?
            .map_err(Into::into),
        (None, Some(_)) => Err(PoolError::NoRuntimeSpecified),
    }
}
