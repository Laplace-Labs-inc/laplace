// SPDX-License-Identifier: Apache-2.0
//! `TrackedStdMutex<T>` — `std::sync::Mutex` 래퍼로 lock/unlock 시 `ProbeEvent`를 자동 전송.
//!
//! [GHOST CONSTRAINT]: `resource_name`은 동일 자원에 대해 항상 동일한 &'static str.
//! `AxiomStepBuilder`는 이름 해시를 `ResourceId`로 고정 매핑하므로 불일치 시 검증 오염.
//!
//! [GHOST CONSTRAINT]: `lock()`은 동기 블로킹 호출이다.
//! tokio async 컨텍스트 내에서 호출하면 tokio 스레드를 블로킹한다.
//! 테스트 목적으로는 short critical section에 한해 허용.

use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, MutexGuard};

#[cfg(feature = "verification")]
use laplace_probe::ProbeEvent;

use crate::session::{current_thread_id, emit};

macro_rules! emit_probe_event {
    ($event:expr) => {
        #[cfg(feature = "verification")]
        {
            emit($event);
        }
    };
}

/// `std::sync::Mutex<T>` 래퍼 — lock/unlock 시 `ProbeEvent`를 자동 전송한다.
///
/// BYOC Phase 2에서 `std::sync::Mutex` 기반 라이브러리 검증에 사용.
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
    /// 이름과 초기값으로 `TrackedStdMutex`를 생성한다.
    pub fn new(value: T, resource_name: &'static str) -> Self {
        Self {
            inner: Mutex::new(value),
            resource_name,
        }
    }

    /// Lock을 동기적으로 획득한다. 획득 후 `ProbeEvent::LockAcquired`를 전송한다.
    ///
    /// [GHOST CONSTRAINT]: 호출 전 OS 스레드에 `set_probe_sender()` +
    /// `set_probe_thread_id()` 가 설정되어 있어야 한다. 미설정 시 no-op (이벤트 전송 안 함).
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

/// RAII 가드 — Drop 시 `ProbeEvent::LockReleased`를 자동 전송한다.
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
