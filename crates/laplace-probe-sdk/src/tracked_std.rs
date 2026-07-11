// SPDX-License-Identifier: Apache-2.0
//! `TrackedStdMutex<T>` — `std::sync::Mutex` wrapper that automatically emits
//! `ProbeEvent` on lock/unlock.
//!
//! [GHOST CONSTRAINT]: `resource_name` must always be the same &'static str for
//! a given resource.
//! Downstream adapters use the name as the stable synchronization resource key.
//!
//! [GHOST CONSTRAINT]: `lock()` is a synchronous blocking call.
//! Calling it in a tokio async context blocks a tokio thread.
//! It is allowed for short critical sections in tests.

use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, MutexGuard};

use crate::ProbeEvent;

use crate::session::current_thread_id;
use crate::session::emit;

macro_rules! emit_probe_event {
    ($event:expr) => {{
        emit($event);
    }};
}

/// `std::sync::Mutex<T>` wrapper that automatically emits `ProbeEvent` on
/// lock/unlock.
///
/// Used to verify `std::sync::Mutex`-based libraries during BYOC Phase 2.
///
/// ```ignore
/// // before
/// struct SharedState { data: std::sync::Mutex<Vec<i64>> }
///
/// // after
/// use laplace_probe_sdk::TrackedStdMutex;
/// struct SharedState { data: TrackedStdMutex<Vec<i64>> }
/// ```
pub struct TrackedStdMutex<T> {
    inner: Mutex<T>,
    resource_name: &'static str,
}

impl<T> TrackedStdMutex<T> {
    /// Creates a `TrackedStdMutex` with a name and initial value.
    pub fn new(value: T, resource_name: &'static str) -> Self {
        Self {
            inner: Mutex::new(value),
            resource_name,
        }
    }

    /// Acquires the lock synchronously and sends `ProbeEvent::LockAcquired`
    /// after acquisition.
    ///
    /// [GHOST CONSTRAINT]: before calling, set `set_probe_sender()` and
    /// `set_probe_thread_id()` on the OS thread. If unset, this is a no-op (no
    /// event is sent).
    ///
    /// # Panics
    ///
    /// Panics if the mutex is poisoned (a thread panicked while holding the lock).
    pub fn lock(&self) -> TrackedStdGuard<'_, T> {
        let thread_id = current_thread_id();
        // 동기 블로킹 획득 — Poison 시 panic (테스트 환경)
        #[allow(clippy::unwrap_used)]
        let guard = self.inner.lock().unwrap();

        // Lock 획득 후 이벤트 전송 (획득 전 전송하면 순서 역전 가능)
        emit_probe_event!(ProbeEvent::LockAcquired {
            thread_id,
            resource: self.resource_name.to_string(),
        });

        TrackedStdGuard {
            inner: guard,
            resource_name: self.resource_name,
            thread_id,
        }
    }
}

/// RAII guard that automatically sends `ProbeEvent::LockReleased` on drop.
#[cfg_attr(not(laplace_private_verification), allow(dead_code))]
pub struct TrackedStdGuard<'a, T> {
    inner: MutexGuard<'a, T>,
    resource_name: &'static str,
    thread_id: u64,
}

impl<T> Deref for TrackedStdGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T> DerefMut for TrackedStdGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T> Drop for TrackedStdGuard<'_, T> {
    fn drop(&mut self) {
        emit_probe_event!(ProbeEvent::LockReleased {
            thread_id: self.thread_id,
            resource: self.resource_name.to_string(),
        });
    }
}
