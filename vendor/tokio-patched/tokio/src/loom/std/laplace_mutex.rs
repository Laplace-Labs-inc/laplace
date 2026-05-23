//! Laplace-instrumented Mutex replacement.
//!
//! `std::sync::Mutex` 기반이지만 Ki-DPOR 이벤트를 방출한다.
//! tokio 내부의 모든 Mutex 연산이 캡처된다.
//!
//! NOTE: std::sync::Condvar 호환성을 위해 MutexGuard는 std::sync::MutexGuard를 직접 사용한다.
//! 따라서 acquire 이벤트만 방출하고, release 이벤트는 방출하지 않는다.
//! (Release를 추적하려면 wrapper type이 필요한데, 이는 Condvar와 호환되지 않음)

use laplace_probe_sdk::{current_thread_id, emit, ProbeEvent};
use std::sync::atomic::{AtomicUsize, Ordering};

static MUTEX_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn alloc_name() -> &'static str {
    let id = MUTEX_COUNTER.fetch_add(1, Ordering::Relaxed);
    Box::leak(format!("tokio_internal_mutex_{}", id).into_boxed_str())
}

fn emit_acquired(name: &'static str) {
    emit(ProbeEvent::LockAcquired {
        thread_id: current_thread_id(),
        resource: name.to_string(),
    });
}

pub(crate) struct Mutex<T> {
    inner: std::sync::Mutex<T>,
    name: std::cell::OnceCell<&'static str>,
}

impl<T> Mutex<T> {
    #[inline]
    pub(crate) fn new(t: T) -> Self {
        Self {
            inner: std::sync::Mutex::new(t),
            name: std::cell::OnceCell::from(alloc_name()),
        }
    }

    /// const context 호환. 이름은 첫 lock() 호출 시 lazy 할당.
    #[inline]
    pub(crate) const fn const_new(t: T) -> Self {
        Self {
            inner: std::sync::Mutex::new(t),
            name: std::cell::OnceCell::new(), // lazy — 첫 lock()에서 초기화
        }
    }

    #[inline]
    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, T> {
        let name = self.name.get_or_init(alloc_name);
        let guard = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        emit_acquired(name);
        guard
    }

    #[inline]
    pub(crate) fn try_lock(&self) -> Option<std::sync::MutexGuard<'_, T>> {
        let name = self.name.get_or_init(alloc_name);
        match self.inner.try_lock() {
            Ok(guard) => {
                emit_acquired(name);
                Some(guard)
            }
            Err(std::sync::TryLockError::Poisoned(p)) => {
                emit_acquired(name);
                Some(p.into_inner())
            }
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
    }

    #[inline]
    pub(crate) fn get_mut(&mut self) -> &mut T {
        self.inner.get_mut().unwrap_or_else(|p| p.into_inner())
    }
}

// Send + Sync: std::sync::Mutex<T>와 동일한 보장
// SAFETY: Mutex<T> has the same safety guarantees as std::sync::Mutex<T>
unsafe impl<T: Send> Send for Mutex<T> {}
// SAFETY: Mutex<T> has the same safety guarantees as std::sync::Mutex<T>
unsafe impl<T: Send> Sync for Mutex<T> {}

impl<T: std::fmt::Debug> std::fmt::Debug for Mutex<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = self.name.get().unwrap_or(&"uninitialized");
        f.debug_struct("Mutex").field("name", name).finish()
    }
}

// Type alias for compatibility with std::sync::MutexGuard
pub(crate) use std::sync::MutexGuard;
