//! `TrackedMutex<T>` — `Mutex` 래퍼로 lock/unlock 시 `ProbeEvent`를 자동 전송.
//!
//! [GHOST CONSTRAINT]: `resource_name`은 동일 자원에 대해 항상 동일한 &'static str.
//! `AxiomStepBuilder`는 이름 해시를 `ResourceId`로 고정 매핑하므로 불일치 시 검증 오염.

use std::ops::{Deref, DerefMut};

use laplace_probe::ProbeEvent;
use tokio::sync::{Mutex, MutexGuard};

use crate::session::{current_thread_id, emit};

/// `tokio::sync::Mutex<T>` 래퍼 — lock/unlock 시 `ProbeEvent`를 자동 전송한다.
///
/// BYOC Phase 1에서 사용자가 `Mutex<T>` 대신 선언하는 유일한 타입 교체.
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
    /// 이름과 초기값으로 `TrackedMutex`를 생성한다.
    pub fn new(value: T, resource_name: &'static str) -> Self {
        Self {
            inner: Mutex::new(value),
            resource_name,
        }
    }

    /// Lock을 비동기로 획득한다. 획득 후 `ProbeEvent::LockAcquired`를 전송한다.
    ///
    /// [GHOST CONSTRAINT]: `lock()` 호출 전 OS 스레드에 `set_probe_sender()` +
    /// `set_probe_thread_id()` 가 설정되어 있어야 한다. 미설정 시 no-op (이벤트 전송 안 함).
    pub async fn lock(&self) -> TrackedGuard<'_, T> {
        let thread_id = current_thread_id();
        let guard = self.inner.lock().await;

        // Lock 획득 후 이벤트 전송 (획득 전 전송하면 순서 역전 가능)
        emit(ProbeEvent::LockAcquired {
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

/// RAII 가드 — Drop 시 `ProbeEvent::LockReleased`를 자동 전송한다.
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
        emit(ProbeEvent::LockReleased {
            thread_id: self.thread_id,
            resource: self.resource_name.to_string(),
        });
    }
}
