// SPDX-License-Identifier: Apache-2.0
//! `TrackedMutex<T>` — `Mutex` wrapper that automatically emits `ProbeEvent` on
//! lock/unlock.
//!
//! [GHOST CONSTRAINT]: `resource_name` must always be the same &'static str for
//! a given resource.
//! Downstream adapters use the name as the stable synchronization resource key.

use std::ops::{Deref, DerefMut};

use crate::ProbeEvent;
use tokio::sync::{Mutex, MutexGuard};

use crate::session::current_thread_id;
use crate::session::emit;

macro_rules! emit_probe_event {
    ($event:expr) => {{
        emit($event);
    }};
}

/// `tokio::sync::Mutex<T>` wrapper that automatically emits `ProbeEvent` on
/// lock/unlock.
///
/// During BYOC Phase 1, this is the only type users replace `Mutex<T>` with.
///
/// ```ignore
/// // before
/// struct AppState { counter: tokio::sync::Mutex<i64> }
///
/// // after
/// use laplace_probe_sdk::TrackedMutex;
/// struct AppState { counter: TrackedMutex<i64> }
/// ```
pub struct TrackedMutex<T> {
    inner: Mutex<T>,
    resource_name: &'static str,
}

impl<T> TrackedMutex<T> {
    /// Creates a `TrackedMutex` with a name and initial value.
    pub fn new(value: T, resource_name: &'static str) -> Self {
        Self {
            inner: Mutex::new(value),
            resource_name,
        }
    }

    /// Alias used by convenience macros.
    pub fn named(value: T, resource_name: &'static str) -> Self {
        Self::new(value, resource_name)
    }

    /// Acquires the lock asynchronously and sends `ProbeEvent::LockAcquired`
    /// after acquisition.
    ///
    /// [GHOST CONSTRAINT]: before calling `lock()`, set `set_probe_sender()` and
    /// `set_probe_thread_id()` on the OS thread. If unset, this is a no-op (no
    /// event is sent).
    pub async fn lock(&self) -> TrackedGuard<'_, T> {
        let thread_id = current_thread_id();
        let guard = self.inner.lock().await;

        // Lock 획득 후 이벤트 전송 (획득 전 전송하면 순서 역전 가능)
        emit_probe_event!(ProbeEvent::LockAcquired {
            thread_id,
            resource: self.resource_name.to_string(),
        });

        TrackedGuard {
            inner: guard,
            resource_name: self.resource_name,
            thread_id,
        }
    }
}

/// RAII guard that automatically sends `ProbeEvent::LockReleased` on drop.
#[cfg_attr(not(laplace_private_verification), allow(dead_code))]
pub struct TrackedGuard<'a, T> {
    inner: MutexGuard<'a, T>,
    resource_name: &'static str,
    thread_id: u64,
}

impl<T> Deref for TrackedGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T> DerefMut for TrackedGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T> Drop for TrackedGuard<'_, T> {
    fn drop(&mut self) {
        emit_probe_event!(ProbeEvent::LockReleased {
            thread_id: self.thread_id,
            resource: self.resource_name.to_string(),
        });
    }
}
