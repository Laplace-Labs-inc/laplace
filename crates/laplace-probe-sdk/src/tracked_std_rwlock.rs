// SPDX-License-Identifier: Apache-2.0
//! `TrackedStdRwLock<T>` — `std::sync::RwLock` wrapper (synchronous version).
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
use std::sync::RwLock;

/// `std::sync::RwLock<T>` wrapper that automatically tracks read/write lock
/// events (synchronous).
pub struct TrackedStdRwLock<T> {
    inner: RwLock<T>,
    resource_name: &'static str,
}

impl<T> TrackedStdRwLock<T> {
    /// Creates a new `TrackedStdRwLock`.
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

    /// Acquires a shared (read) lock. Multiple threads may hold it concurrently.
    pub fn read(&self) -> TrackedStdRwLockReadGuard<'_, T> {
        let thread_id = current_thread_id();
        // SAFETY: Poison handling — lock() may panic on poisoned RwLock, but this
        // is expected behavior per Rust stdlib semantics.
        #[allow(clippy::unwrap_used)]
        let guard = self.inner.read().unwrap();
        emit_probe_event!(ProbeEvent::RwLockReadAcquired {
            thread_id,
            resource: self.resource_name.to_string(),
        });
        TrackedStdRwLockReadGuard {
            inner: guard,
            resource_name: self.resource_name,
            thread_id,
        }
    }

    /// Acquires an exclusive (write) lock. Only one thread may hold it at a time.
    pub fn write(&self) -> TrackedStdRwLockWriteGuard<'_, T> {
        let thread_id = current_thread_id();
        // SAFETY: Poison handling — lock() may panic on poisoned RwLock, but this
        // is expected behavior per Rust stdlib semantics.
        #[allow(clippy::unwrap_used)]
        let guard = self.inner.write().unwrap();
        emit_probe_event!(ProbeEvent::RwLockWriteAcquired {
            thread_id,
            resource: self.resource_name.to_string(),
        });
        TrackedStdRwLockWriteGuard {
            inner: guard,
            resource_name: self.resource_name,
            thread_id,
        }
    }
}

/// Read guard for `TrackedStdRwLock`.
///
/// [GHOST CONSTRAINT]: no `DerefMut` (read-only).
#[cfg_attr(not(laplace_private_verification), allow(dead_code))]
pub struct TrackedStdRwLockReadGuard<'a, T> {
    inner: std::sync::RwLockReadGuard<'a, T>,
    resource_name: &'static str,
    thread_id: u64,
}

impl<T> Deref for TrackedStdRwLockReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T> Drop for TrackedStdRwLockReadGuard<'_, T> {
    fn drop(&mut self) {
        emit_probe_event!(ProbeEvent::RwLockReadReleased {
            thread_id: self.thread_id,
            resource: self.resource_name.to_string(),
        });
    }
}

/// Write guard for `TrackedStdRwLock`.
#[cfg_attr(not(laplace_private_verification), allow(dead_code))]
pub struct TrackedStdRwLockWriteGuard<'a, T> {
    inner: std::sync::RwLockWriteGuard<'a, T>,
    resource_name: &'static str,
    thread_id: u64,
}

impl<T> Deref for TrackedStdRwLockWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T> DerefMut for TrackedStdRwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T> Drop for TrackedStdRwLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        emit_probe_event!(ProbeEvent::RwLockWriteReleased {
            thread_id: self.thread_id,
            resource: self.resource_name.to_string(),
        });
    }
}
