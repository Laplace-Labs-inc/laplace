// SPDX-License-Identifier: Apache-2.0
//! `TrackedRwLock<T>` — `tokio::sync::RwLock` 래퍼.
//!
//! read() → RwLockReadAcquired, write() → RwLockWriteAcquired 이벤트 전송.
//! Guard drop 시 각각 RwLockReadReleased / RwLockWriteReleased 이벤트 전송.

use crate::session::{current_thread_id, emit};
use laplace_probe::ProbeEvent;
use std::ops::{Deref, DerefMut};
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

/// `tokio::sync::RwLock<T>` 래퍼로, 읽기/쓰기 락 이벤트를 자동으로 추적한다.
pub struct TrackedRwLock<T> {
    inner: RwLock<T>,
    resource_name: &'static str,
}

impl<T> TrackedRwLock<T> {
    /// 새로운 TrackedRwLock을 생성한다.
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

    /// Alias used by convenience macros.
    pub fn named(value: T, resource_name: &'static str) -> Self {
        Self::new(value, resource_name)
    }

    /// 공유 (읽기) 락을 획득한다. 여러 스레드가 동시에 보유할 수 있다.
    pub async fn read(&self) -> TrackedRwLockReadGuard<'_, T> {
        let thread_id = current_thread_id();
        let guard = self.inner.read().await;
        emit(ProbeEvent::RwLockReadAcquired {
            thread_id,
            resource: self.resource_name.to_string(),
        });
        TrackedRwLockReadGuard {
            inner: guard,
            resource_name: self.resource_name,
            thread_id,
        }
    }

    /// 배타적 (쓰기) 락을 획득한다. 한 번에 하나의 스레드만 보유할 수 있다.
    pub async fn write(&self) -> TrackedRwLockWriteGuard<'_, T> {
        let thread_id = current_thread_id();
        let guard = self.inner.write().await;
        emit(ProbeEvent::RwLockWriteAcquired {
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

/// TrackedRwLock의 읽기 가드.
///
/// [GHOST CONSTRAINT]: DerefMut 없음 (읽기 전용).
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
        emit(ProbeEvent::RwLockReadReleased {
            thread_id: self.thread_id,
            resource: self.resource_name.to_string(),
        });
    }
}

/// TrackedRwLock의 쓰기 가드.
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
        emit(ProbeEvent::RwLockWriteReleased {
            thread_id: self.thread_id,
            resource: self.resource_name.to_string(),
        });
    }
}
