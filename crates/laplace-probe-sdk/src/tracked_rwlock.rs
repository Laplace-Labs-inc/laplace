// SPDX-License-Identifier: Apache-2.0
//! `TrackedRwLock<T>` — `tokio::sync::RwLock` wrapper.
//!
//! `read()` emits `RwLockReadAcquired`, and `write()` emits
//! `RwLockWriteAcquired`.
//! Guard drop emits `RwLockReadReleased` or `RwLockWriteReleased` respectively.

use crate::session::current_thread_id;
use crate::session::emit;

macro_rules! emit_probe_event {
    ($event:expr) => {{
        emit($event);
    }};
}
use crate::ProbeEvent;
use std::ops::{Deref, DerefMut};
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

/// `tokio::sync::RwLock<T>` wrapper that automatically tracks read/write lock events.
pub struct TrackedRwLock<T> {
    inner: RwLock<T>,
    resource_name: &'static str,
}

impl<T> TrackedRwLock<T> {
    /// Creates a new `TrackedRwLock`.
    ///
    /// # Arguments
    ///
    /// * `value` — value to protect
    /// * `resource_name` — resource name for engine tracking (&'static str)
    pub fn new(value: T, resource_name: &'static str) -> Self {
        Self {
            inner: RwLock::new(value),
            resource_name,
        }
    }

    /// Alias used by convenience macros.
    pub fn named(value: T, resource_name: &'static str) -> Self {
        Self::new(value, resource_name)
    }

    /// Acquires a shared (read) lock. Multiple threads may hold it concurrently.
    pub async fn read(&self) -> TrackedRwLockReadGuard<'_, T> {
        let thread_id = current_thread_id();
        let guard = self.inner.read().await;
        emit_probe_event!(ProbeEvent::RwLockReadAcquired {
            thread_id,
            resource: self.resource_name.to_string(),
        });
        TrackedRwLockReadGuard {
            inner: guard,
            resource_name: self.resource_name,
            thread_id,
        }
    }

    /// Acquires an exclusive (write) lock. Only one thread may hold it at a time.
    pub async fn write(&self) -> TrackedRwLockWriteGuard<'_, T> {
        let thread_id = current_thread_id();
        let guard = self.inner.write().await;
        emit_probe_event!(ProbeEvent::RwLockWriteAcquired {
            thread_id,
            resource: self.resource_name.to_string(),
        });
        TrackedRwLockWriteGuard {
            inner: guard,
            resource_name: self.resource_name,
            thread_id,
        }
    }
}

/// Read guard for `TrackedRwLock`.
///
/// [GHOST CONSTRAINT]: no `DerefMut` (read-only).
#[cfg_attr(not(laplace_private_verification), allow(dead_code))]
pub struct TrackedRwLockReadGuard<'a, T> {
    inner: RwLockReadGuard<'a, T>,
    resource_name: &'static str,
    thread_id: u64,
}

impl<T> Deref for TrackedRwLockReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T> Drop for TrackedRwLockReadGuard<'_, T> {
    fn drop(&mut self) {
        emit_probe_event!(ProbeEvent::RwLockReadReleased {
            thread_id: self.thread_id,
            resource: self.resource_name.to_string(),
        });
    }
}

/// Write guard for `TrackedRwLock`.
#[cfg_attr(not(laplace_private_verification), allow(dead_code))]
pub struct TrackedRwLockWriteGuard<'a, T> {
    inner: RwLockWriteGuard<'a, T>,
    resource_name: &'static str,
    thread_id: u64,
}

impl<T> Deref for TrackedRwLockWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T> DerefMut for TrackedRwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T> Drop for TrackedRwLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        emit_probe_event!(ProbeEvent::RwLockWriteReleased {
            thread_id: self.thread_id,
            resource: self.resource_name.to_string(),
        });
    }
}
