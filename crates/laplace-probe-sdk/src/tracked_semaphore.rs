// SPDX-License-Identifier: Apache-2.0
//! `TrackedSemaphore` — `tokio::sync::Semaphore` wrapper.
//!
//! `acquire` emits `SemaphoreAcquired`, and permit drop emits
//! `SemaphoreReleased`.

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

/// `tokio::sync::Semaphore` wrapper that automatically tracks acquire/release events.
pub struct TrackedSemaphore {
    inner: Arc<Semaphore>,
    resource_name: &'static str,
}

impl TrackedSemaphore {
    /// Creates a new `TrackedSemaphore`.
    ///
    /// # Arguments
    ///
    /// * `permits` — initial permit count
    /// * `resource_name` — resource name for engine tracking (&'static str)
    pub fn new(permits: usize, resource_name: &'static str) -> Self {
        Self {
            inner: Arc::new(Semaphore::new(permits)),
            resource_name,
        }
    }

    /// Acquires a semaphore permit.
    ///
    /// # Panics
    ///
    /// Panics if the semaphore is closed.
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

    /// Returns the current number of available permits.
    pub fn available_permits(&self) -> usize {
        self.inner.available_permits()
    }
}

/// A permit acquired from `TrackedSemaphore`.
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
