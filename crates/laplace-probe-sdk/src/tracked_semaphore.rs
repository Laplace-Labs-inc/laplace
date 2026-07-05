// SPDX-License-Identifier: Apache-2.0
//! `TrackedSemaphore` — `tokio::sync::Semaphore` 래퍼.
//!
//! acquire → SemaphoreAcquired, Permit drop → SemaphoreReleased 이벤트 전송.

use crate::session::current_thread_id;
use crate::session::emit;

macro_rules! emit_probe_event {
    ($event:expr) => {{
        emit($event);
    }};
}
use crate::ProbeEvent;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// `tokio::sync::Semaphore` 래퍼로, 세마포어 획득/해제 이벤트를 자동으로 추적한다.
pub struct TrackedSemaphore {
    inner: Arc<Semaphore>,
    resource_name: &'static str,
}

impl TrackedSemaphore {
    /// 새로운 TrackedSemaphore를 생성한다.
    ///
    /// # Arguments
    ///
    /// * `permits` — 초기 permit 개수
    /// * `resource_name` — 엔진 추적용 리소스 이름 (&'static str)
    pub fn new(permits: usize, resource_name: &'static str) -> Self {
        Self {
            inner: Arc::new(Semaphore::new(permits)),
            resource_name,
        }
    }

    /// Semaphore permit을 획득한다.
    ///
    /// # Panics
    ///
    /// Semaphore가 closed된 경우 panic한다.
    pub async fn acquire(&self) -> TrackedSemaphorePermit {
        let thread_id = current_thread_id();
        // SAFETY: We expect the semaphore to be open; if it's closed, panicking
        // is the intended behavior per tokio semantics.
        #[allow(clippy::expect_used)]
        let permit = self
            .inner
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore closed");
        emit_probe_event!(ProbeEvent::SemaphoreAcquired {
            thread_id,
            resource: self.resource_name.to_string(),
        });
        TrackedSemaphorePermit {
            _permit: permit,
            resource_name: self.resource_name,
            thread_id,
        }
    }

    /// 현재 available permits 개수를 반환한다.
    pub fn available_permits(&self) -> usize {
        self.inner.available_permits()
    }
}

/// TrackedSemaphore에서 획득한 permit.
#[cfg_attr(not(laplace_private_verification), allow(dead_code))]
pub struct TrackedSemaphorePermit {
    _permit: OwnedSemaphorePermit,
    resource_name: &'static str,
    thread_id: u64,
}

impl Drop for TrackedSemaphorePermit {
    fn drop(&mut self) {
        emit_probe_event!(ProbeEvent::SemaphoreReleased {
            thread_id: self.thread_id,
            resource: self.resource_name.to_string(),
        });
    }
}
