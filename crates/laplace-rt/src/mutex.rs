// SPDX-License-Identifier: Apache-2.0
//! `std::sync::Mutex`-compatible model mutex.

use std::ops::{Deref, DerefMut};
use std::sync::{
    LockResult, Mutex as StdMutex, MutexGuard as StdMutexGuard, PoisonError, TryLockError,
    TryLockResult,
};

use crate::hooks::{lock_hook, next_lock_resource_id, spawn_hook};

/// `std::sync::Mutex<T>` compatible model mutex for annotated code.
pub struct ModelMutex<T: ?Sized> {
    resource: u64,
    inner: StdMutex<T>,
}

impl<T> ModelMutex<T> {
    /// Creates a new model mutex with a distinct process-local resource id.
    pub fn new(t: T) -> Self {
        Self {
            resource: next_lock_resource_id(),
            inner: StdMutex::new(t),
        }
    }
}

impl<T: ?Sized> ModelMutex<T> {
    /// Acquires the mutex.
    ///
    /// The signature mirrors `std::sync::Mutex::lock`, allowing annotated
    /// source to keep `.lock().unwrap()` unchanged. When a hook is installed,
    /// the acquire boundary is reported before the underlying lock attempt and
    /// the release boundary is reported when the returned guard is dropped.
    ///
    /// # Errors
    ///
    /// Returns a [`PoisonError`] if a previous holder panicked.
    ///
    /// # Panics
    ///
    /// Panics if an installed engine hook grants a lock that is still contended.
    pub fn lock(&self) -> LockResult<ModelMutexGuard<'_, T>> {
        let hook = lock_hook();
        if let Some(hook) = &hook {
            hook.acquire(self.resource);
        }

        if hook.is_some() && spawn_hook().is_some() {
            return self.inner.try_lock().map_or_else(
                |err| match err {
                    TryLockError::Poisoned(poisoned) => Err(PoisonError::new(ModelMutexGuard {
                        inner: Some(poisoned.into_inner()),
                        resource: self.resource,
                    })),
                    TryLockError::WouldBlock => {
                        panic!("laplace-rt lock hook granted a contended mutex")
                    }
                },
                |inner| {
                    Ok(ModelMutexGuard {
                        inner: Some(inner),
                        resource: self.resource,
                    })
                },
            );
        }

        self.inner
            .lock()
            .map(|inner| ModelMutexGuard {
                inner: Some(inner),
                resource: self.resource,
            })
            .map_err(|err| {
                PoisonError::new(ModelMutexGuard {
                    inner: Some(err.into_inner()),
                    resource: self.resource,
                })
            })
    }

    /// Attempts to acquire the mutex without blocking.
    ///
    /// Mirrors `std::sync::Mutex::try_lock`. A successful (or poisoned-but-held)
    /// acquisition reports one acquire boundary to an installed hook; a
    /// `WouldBlock` failure reports nothing, since a non-blocking failure holds
    /// no resource and cannot participate in a wait cycle.
    ///
    /// # Errors
    ///
    /// Returns [`TryLockError::WouldBlock`] if the lock is already held, or
    /// [`TryLockError::Poisoned`] if a previous holder panicked.
    pub fn try_lock(&self) -> TryLockResult<ModelMutexGuard<'_, T>> {
        match self.inner.try_lock() {
            Ok(inner) => {
                if let Some(hook) = lock_hook() {
                    hook.acquire(self.resource);
                }
                Ok(ModelMutexGuard {
                    inner: Some(inner),
                    resource: self.resource,
                })
            }
            Err(TryLockError::WouldBlock) => Err(TryLockError::WouldBlock),
            Err(TryLockError::Poisoned(poisoned)) => {
                if let Some(hook) = lock_hook() {
                    hook.acquire(self.resource);
                }
                Err(TryLockError::Poisoned(PoisonError::new(ModelMutexGuard {
                    inner: Some(poisoned.into_inner()),
                    resource: self.resource,
                })))
            }
        }
    }
}

/// Guard returned by [`ModelMutex::lock`].
pub struct ModelMutexGuard<'a, T: ?Sized> {
    inner: Option<StdMutexGuard<'a, T>>,
    resource: u64,
}

impl<T: ?Sized> Deref for ModelMutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner.as_deref().expect("model mutex guard is present")
    }
}

impl<T: ?Sized> DerefMut for ModelMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner
            .as_deref_mut()
            .expect("model mutex guard is present")
    }
}

impl<T: ?Sized> Drop for ModelMutexGuard<'_, T> {
    fn drop(&mut self) {
        if self.inner.is_some() && !std::thread::panicking() {
            if let Some(hook) = lock_hook() {
                hook.release(self.resource);
            }
        }
    }
}
