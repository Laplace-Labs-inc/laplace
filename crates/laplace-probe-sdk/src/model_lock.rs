// SPDX-License-Identifier: Apache-2.0
//! Probe emission hook for `laplace_model_rt::ModelMutex`.

use std::sync::Arc;

use crate::session::{current_thread_id, emit};
use crate::ProbeEvent;

/// Probe hook that emits lock-order events for annotated `std::sync::Mutex`.
pub struct ProbeLockHook;

impl laplace_model_rt::LockHook for ProbeLockHook {
    fn acquire(&self, resource: u64) {
        emit(ProbeEvent::LockAcquired {
            thread_id: current_thread_id(),
            resource: resource_name(resource),
        });
    }

    fn release(&self, resource: u64) {
        emit(ProbeEvent::LockReleased {
            thread_id: current_thread_id(),
            resource: resource_name(resource),
        });
    }
}

/// Installs the process-local probe lock hook used by free-tier tests.
pub fn install_probe_lock_hook() {
    laplace_model_rt::install_lock_hook(Arc::new(ProbeLockHook));
}

fn resource_name(resource: u64) -> String {
    format!("model_mutex:{resource}")
}
