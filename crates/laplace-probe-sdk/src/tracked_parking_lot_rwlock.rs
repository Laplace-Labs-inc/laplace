// SPDX-License-Identifier: Apache-2.0
//! `TrackedParkingLotRwLock<T>` — `parking_lot::RwLock` wrapper.
//!
//! Emits the same events as `TrackedStdRwLock`, while using
//! `parking_lot::RwLock` to support read reentrancy.
//! Used to patch `parking_lot`-based crates such as DashMap.

use crate::session::current_thread_id;
use crate::session::emit;

macro_rules! emit_probe_event {
    ($event:expr) => {{
        emit($event);
    }};
}
use crate::ProbeEvent;
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::ops::{Deref, DerefMut};

/// `parking_lot::RwLock<T>` wrapper with read reentrancy and engine event emission.
pub struct TrackedParkingLotRwLock<T> {
    inner: RwLock<T>,
    resource_name: &'static str,
}

impl<T> TrackedParkingLotRwLock<T> {
    /// Creates a new `TrackedParkingLotRwLock`.
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

    /// Acquires a shared (read) lock with read reentrancy support.
    pub fn read(&self) -> TrackedParkingLotRwLockReadGuard<'_, T> {
        let thread_id = current_thread_id();
        let guard = self.inner.read();
        emit_probe_event!(ProbeEvent::RwLockReadAcquired {
            thread_id,
            resource: self.resource_name.to_string(),
        });
        TrackedParkingLotRwLockReadGuard {
            inner: guard,
            resource_name: self.resource_name,
            thread_id,
        }
    }

    /// Acquires an exclusive (write) lock.
    pub fn write(&self) -> TrackedParkingLotRwLockWriteGuard<'_, T> {
        let thread_id = current_thread_id();
        let guard = self.inner.write();
        emit_probe_event!(ProbeEvent::RwLockWriteAcquired {
            thread_id,
            resource: self.resource_name.to_string(),
        });
        TrackedParkingLotRwLockWriteGuard {
            inner: guard,
            resource_name: self.resource_name,
            thread_id,
        }
    }

    /// Returns a raw pointer to the inner value, bypassing the lock.
    ///
    /// # Safety
    /// Caller must ensure exclusive access or that no mutable references exist.
    pub fn data_ptr(&self) -> *mut T {
        self.inner.data_ptr()
    }

    /// Attempts a non-blocking read.
    pub fn try_read(&self) -> Option<TrackedParkingLotRwLockReadGuard<'_, T>> {
        let thread_id = current_thread_id();
        self.inner.try_read().map(|guard| {
            emit_probe_event!(ProbeEvent::RwLockReadAcquired {
                thread_id,
                resource: self.resource_name.to_string(),
            });
            TrackedParkingLotRwLockReadGuard {
                inner: guard,
                resource_name: self.resource_name,
                thread_id,
            }
        })
    }

    /// Attempts a non-blocking write.
    pub fn try_write(&self) -> Option<TrackedParkingLotRwLockWriteGuard<'_, T>> {
        let thread_id = current_thread_id();
        self.inner.try_write().map(|guard| {
            emit_probe_event!(ProbeEvent::RwLockWriteAcquired {
                thread_id,
                resource: self.resource_name.to_string(),
            });
            TrackedParkingLotRwLockWriteGuard {
                inner: guard,
                resource_name: self.resource_name,
                thread_id,
            }
        })
    }
}

/// Read guard for `TrackedParkingLotRwLock`.
///
/// [GHOST CONSTRAINT]: no `DerefMut` (read-only).
#[cfg_attr(not(laplace_private_verification), allow(dead_code))]
pub struct TrackedParkingLotRwLockReadGuard<'a, T> {
    inner: RwLockReadGuard<'a, T>,
    resource_name: &'static str,
    thread_id: u64,
}

impl<T> Deref for TrackedParkingLotRwLockReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T> Drop for TrackedParkingLotRwLockReadGuard<'_, T> {
    fn drop(&mut self) {
        emit_probe_event!(ProbeEvent::RwLockReadReleased {
            thread_id: self.thread_id,
            resource: self.resource_name.to_string(),
        });
    }
}

/// Write guard for `TrackedParkingLotRwLock`.
#[cfg_attr(not(laplace_private_verification), allow(dead_code))]
pub struct TrackedParkingLotRwLockWriteGuard<'a, T> {
    inner: RwLockWriteGuard<'a, T>,
    resource_name: &'static str,
    thread_id: u64,
}

impl<T> Deref for TrackedParkingLotRwLockWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T> DerefMut for TrackedParkingLotRwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T> Drop for TrackedParkingLotRwLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        emit_probe_event!(ProbeEvent::RwLockWriteReleased {
            thread_id: self.thread_id,
            resource: self.resource_name.to_string(),
        });
    }
}
