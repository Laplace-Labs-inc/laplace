// SPDX-License-Identifier: Apache-2.0
//! `TrackedParkingLotRwLock<T>` — `parking_lot::RwLock` 래퍼.
//!
//! `TrackedStdRwLock`과 동일한 이벤트를 방출하되,
//! `parking_lot::RwLock`을 사용하여 read 재진입을 지원한다.
//! DashMap 등 `parking_lot` 기반 크레이트 패치에 사용.

use crate::session::{current_thread_id, emit};
use laplace_probe::ProbeEvent;
use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::ops::{Deref, DerefMut};

/// `parking_lot::RwLock<T>` 래퍼. read 재진입 지원 + Ki-DPOR 이벤트 방출.
pub struct TrackedParkingLotRwLock<T> {
    inner: RwLock<T>,
    resource_name: &'static str,
}

impl<T> TrackedParkingLotRwLock<T> {
    /// 새로운 TrackedParkingLotRwLock을 생성한다.
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

    /// 공유 (읽기) 락을 획득한다. read 재진입 지원.
    pub fn read(&self) -> TrackedParkingLotRwLockReadGuard<'_, T> {
        let thread_id = current_thread_id();
        let guard = self.inner.read();
        emit(ProbeEvent::RwLockReadAcquired {
            thread_id,
            resource: self.resource_name.to_string(),
        });
        TrackedParkingLotRwLockReadGuard {
            inner: guard,
            resource_name: self.resource_name,
            thread_id,
        }
    }

    /// 배타적 (쓰기) 락을 획득한다.
    pub fn write(&self) -> TrackedParkingLotRwLockWriteGuard<'_, T> {
        let thread_id = current_thread_id();
        let guard = self.inner.write();
        emit(ProbeEvent::RwLockWriteAcquired {
            thread_id,
            resource: self.resource_name.to_string(),
        });
        TrackedParkingLotRwLockWriteGuard {
            inner: guard,
            resource_name: self.resource_name,
            thread_id,
        }
    }

    /// Non-blocking read 시도.
    pub fn try_read(&self) -> Option<TrackedParkingLotRwLockReadGuard<'_, T>> {
        let thread_id = current_thread_id();
        self.inner.try_read().map(|guard| {
            emit(ProbeEvent::RwLockReadAcquired {
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

    /// Non-blocking write 시도.
    pub fn try_write(&self) -> Option<TrackedParkingLotRwLockWriteGuard<'_, T>> {
        let thread_id = current_thread_id();
        self.inner.try_write().map(|guard| {
            emit(ProbeEvent::RwLockWriteAcquired {
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

/// TrackedParkingLotRwLock의 읽기 가드.
///
/// [GHOST CONSTRAINT]: DerefMut 없음 (읽기 전용).
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
        emit(ProbeEvent::RwLockReadReleased {
            thread_id: self.thread_id,
            resource: self.resource_name.to_string(),
        });
    }
}

/// TrackedParkingLotRwLock의 쓰기 가드.
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
        emit(ProbeEvent::RwLockWriteReleased {
            thread_id: self.thread_id,
            resource: self.resource_name.to_string(),
        });
    }
}
