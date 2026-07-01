// SPDX-License-Identifier: Apache-2.0
//! `std::sync::RwLock`-compatible model reader-writer lock.
//!
//! Both [`ModelRwLock::read`] and [`ModelRwLock::write`] route through the
//! *same exclusive* acquire/release boundary as [`ModelMutex`](crate::mutex).
//! The frozen engine models locks as exclusive resources, so a read lock is
//! reported as an exclusive acquisition: this is *sound* for the
//! deadlock/liveness oracle (readers are serialized and never form a false wait
//! cycle) but concurrent reader interleavings are not explored.

use std::ops::{Deref, DerefMut};
use std::sync::{
    LockResult, PoisonError, RwLock as StdRwLock, RwLockReadGuard as StdRwLockReadGuard,
    RwLockWriteGuard as StdRwLockWriteGuard, TryLockError, TryLockResult,
};

use crate::hooks::{lock_hook, next_lock_resource_id};

/// `std::sync::RwLock<T>` compatible model rwlock for annotated code.
pub struct ModelRwLock<T: ?Sized> {
    resource: u64,
    inner: StdRwLock<T>,
}

impl<T> ModelRwLock<T> {
    /// Creates a new model rwlock with a distinct process-local resource id.
    pub fn new(t: T) -> Self {
        Self {
            resource: next_lock_resource_id(),
            inner: StdRwLock::new(t),
        }
    }
}

impl<T: ?Sized> ModelRwLock<T> {
    /// Acquires a read (shared) lock, reported as an exclusive acquire boundary.
    ///
    /// # Errors
    ///
    /// Returns a [`PoisonError`] if a writer panicked while holding the lock.
    pub fn read(&self) -> LockResult<ModelRwLockReadGuard<'_, T>> {
        let hook = lock_hook();
        if let Some(hook) = &hook {
            hook.acquire(self.resource);
        }

        if hook.is_some() {
            return self.inner.try_read().map_or_else(
                |err| match err {
                    TryLockError::Poisoned(poisoned) => {
                        Err(PoisonError::new(ModelRwLockReadGuard {
                            inner: Some(poisoned.into_inner()),
                            resource: self.resource,
                        }))
                    }
                    TryLockError::WouldBlock => {
                        panic!("laplace-rt lock hook granted a contended rwlock (read)")
                    }
                },
                |inner| {
                    Ok(ModelRwLockReadGuard {
                        inner: Some(inner),
                        resource: self.resource,
                    })
                },
            );
        }

        self.inner
            .read()
            .map(|inner| ModelRwLockReadGuard {
                inner: Some(inner),
                resource: self.resource,
            })
            .map_err(|err| {
                PoisonError::new(ModelRwLockReadGuard {
                    inner: Some(err.into_inner()),
                    resource: self.resource,
                })
            })
    }

    /// Acquires a write (exclusive) lock, reported as an exclusive acquire.
    ///
    /// # Errors
    ///
    /// Returns a [`PoisonError`] if a previous holder panicked.
    pub fn write(&self) -> LockResult<ModelRwLockWriteGuard<'_, T>> {
        let hook = lock_hook();
        if let Some(hook) = &hook {
            hook.acquire(self.resource);
        }

        if hook.is_some() {
            return self.inner.try_write().map_or_else(
                |err| match err {
                    TryLockError::Poisoned(poisoned) => {
                        Err(PoisonError::new(ModelRwLockWriteGuard {
                            inner: Some(poisoned.into_inner()),
                            resource: self.resource,
                        }))
                    }
                    TryLockError::WouldBlock => {
                        panic!("laplace-rt lock hook granted a contended rwlock (write)")
                    }
                },
                |inner| {
                    Ok(ModelRwLockWriteGuard {
                        inner: Some(inner),
                        resource: self.resource,
                    })
                },
            );
        }

        self.inner
            .write()
            .map(|inner| ModelRwLockWriteGuard {
                inner: Some(inner),
                resource: self.resource,
            })
            .map_err(|err| {
                PoisonError::new(ModelRwLockWriteGuard {
                    inner: Some(err.into_inner()),
                    resource: self.resource,
                })
            })
    }

    /// Attempts a non-blocking read lock.
    ///
    /// A successful acquisition reports one acquire boundary; a `WouldBlock`
    /// failure reports nothing.
    ///
    /// # Errors
    ///
    /// Returns [`TryLockError::WouldBlock`] if a writer holds the lock, or
    /// [`TryLockError::Poisoned`] if a writer panicked while holding it.
    pub fn try_read(&self) -> TryLockResult<ModelRwLockReadGuard<'_, T>> {
        match self.inner.try_read() {
            Ok(inner) => {
                if let Some(hook) = lock_hook() {
                    hook.acquire(self.resource);
                }
                Ok(ModelRwLockReadGuard {
                    inner: Some(inner),
                    resource: self.resource,
                })
            }
            Err(TryLockError::WouldBlock) => Err(TryLockError::WouldBlock),
            Err(TryLockError::Poisoned(poisoned)) => {
                if let Some(hook) = lock_hook() {
                    hook.acquire(self.resource);
                }
                Err(TryLockError::Poisoned(PoisonError::new(
                    ModelRwLockReadGuard {
                        inner: Some(poisoned.into_inner()),
                        resource: self.resource,
                    },
                )))
            }
        }
    }

    /// Attempts a non-blocking write lock.
    ///
    /// A successful acquisition reports one acquire boundary; a `WouldBlock`
    /// failure reports nothing.
    ///
    /// # Errors
    ///
    /// Returns [`TryLockError::WouldBlock`] if the lock is held, or
    /// [`TryLockError::Poisoned`] if a previous holder panicked.
    pub fn try_write(&self) -> TryLockResult<ModelRwLockWriteGuard<'_, T>> {
        match self.inner.try_write() {
            Ok(inner) => {
                if let Some(hook) = lock_hook() {
                    hook.acquire(self.resource);
                }
                Ok(ModelRwLockWriteGuard {
                    inner: Some(inner),
                    resource: self.resource,
                })
            }
            Err(TryLockError::WouldBlock) => Err(TryLockError::WouldBlock),
            Err(TryLockError::Poisoned(poisoned)) => {
                if let Some(hook) = lock_hook() {
                    hook.acquire(self.resource);
                }
                Err(TryLockError::Poisoned(PoisonError::new(
                    ModelRwLockWriteGuard {
                        inner: Some(poisoned.into_inner()),
                        resource: self.resource,
                    },
                )))
            }
        }
    }
}

/// Read guard returned by [`ModelRwLock::read`].
pub struct ModelRwLockReadGuard<'a, T: ?Sized> {
    inner: Option<StdRwLockReadGuard<'a, T>>,
    resource: u64,
}

impl<T: ?Sized> Deref for ModelRwLockReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner
            .as_deref()
            .expect("model rwlock read guard is present")
    }
}

impl<T: ?Sized> Drop for ModelRwLockReadGuard<'_, T> {
    fn drop(&mut self) {
        if self.inner.is_some() && !std::thread::panicking() {
            if let Some(hook) = lock_hook() {
                hook.release(self.resource);
            }
        }
    }
}

/// Write guard returned by [`ModelRwLock::write`].
pub struct ModelRwLockWriteGuard<'a, T: ?Sized> {
    inner: Option<StdRwLockWriteGuard<'a, T>>,
    resource: u64,
}

impl<T: ?Sized> Deref for ModelRwLockWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner
            .as_deref()
            .expect("model rwlock write guard is present")
    }
}

impl<T: ?Sized> DerefMut for ModelRwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner
            .as_deref_mut()
            .expect("model rwlock write guard is present")
    }
}

impl<T: ?Sized> Drop for ModelRwLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        if self.inner.is_some() && !std::thread::panicking() {
            if let Some(hook) = lock_hook() {
                hook.release(self.resource);
            }
        }
    }
}
