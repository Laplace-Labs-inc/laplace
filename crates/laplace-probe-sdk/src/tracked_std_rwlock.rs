// SPDX-License-Identifier: Apache-2.0
//! `TrackedStdRwLock<T>` — `std::sync::RwLock` 래퍼 (동기 버전).
//!
//! read() → RwLockReadAcquired, write() → RwLockWriteAcquired 이벤트 전송.
//! Guard drop 시 각각 RwLockReadReleased / RwLockWriteReleased 이벤트 전송.

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

/// `std::sync::RwLock<T>` 래퍼로, 읽기/쓰기 락 이벤트를 자동으로 추적한다 (동기).
pub struct TrackedStdRwLock<T> {
    inner: RwLock<T>,
    resource_name: &'static str,
}

impl<T> TrackedStdRwLock<T> {
    /// 새로운 TrackedStdRwLock을 생성한다.
    ///
    /// # Arguments
    ///
    /// * `value` — 보호할 값
    /// * `resource_name` — Ki-DPOR 추적용 리소스 이름 (&'static str)
    pub fn new(value: T, resource_name: &'static str) -> Self {
        Self {
            inner: RwLock::new(value),
            resource_name,
        }
    }

    /// 공유 (읽기) 락을 획득한다. 여러 스레드가 동시에 보유할 수 있다.
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

    /// 배타적 (쓰기) 락을 획득한다. 한 번에 하나의 스레드만 보유할 수 있다.
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

/// TrackedStdRwLock의 읽기 가드.
///
/// [GHOST CONSTRAINT]: DerefMut 없음 (읽기 전용).
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

/// TrackedStdRwLock의 쓰기 가드.
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
