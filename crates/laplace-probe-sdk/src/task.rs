// SPDX-License-Identifier: Apache-2.0
//! Probe observation for the native TaskSet surface.

use std::sync::Arc;

use crate::session::{
    clear_current_task_id, current_task_id, emit, set_current_task_id, set_probe_thread_id,
};
use crate::ProbeEvent;

/// Probe hook that projects TaskSet lifecycle callbacks into existing async
/// ProbeEvent variants.
pub struct ProbeTaskHook;

impl laplace_rt::TaskObserverHook for ProbeTaskHook {
    fn task_registered(&self, task: u64) {
        emit(ProbeEvent::TaskSpawned {
            task_id: task,
            parent_task_id: None,
            source_location: None,
        });
    }

    fn dynamic_task_spawned(&self, task: u64) {
        emit(ProbeEvent::TaskSpawned {
            task_id: task,
            parent_task_id: current_task_id(),
            source_location: None,
        });
    }

    fn poll_started(&self, task: u64, attempt: u64) {
        set_current_task_id(task);
        set_probe_thread_id(task);
        emit(ProbeEvent::TaskPolled {
            task_id: task,
            poll_attempt_id: attempt,
        });
    }

    fn poll_completed(&self, task: u64, attempt: u64, outcome: laplace_rt::TaskPollOutcome) {
        match outcome {
            laplace_rt::TaskPollOutcome::Pending => {
                emit(ProbeEvent::FuturePending {
                    task_id: task,
                    future_id: None,
                    poll_attempt_id: attempt,
                });
            }
            laplace_rt::TaskPollOutcome::Ready => {
                emit(ProbeEvent::FutureReady {
                    task_id: task,
                    future_id: None,
                    poll_attempt_id: attempt,
                });
            }
            laplace_rt::TaskPollOutcome::Panicked => {}
        }
        clear_current_task_id();
    }

    fn task_completed(&self, task: u64) {
        clear_current_task_id();
        emit(ProbeEvent::TaskCompleted { task_id: task });
    }
}

/// Installs the process-local probe task hook.
pub fn install_probe_task_hook() {
    laplace_rt::install_task_observer_hook(Arc::new(ProbeTaskHook));
}

/// Runs all registered tasks once on a current-thread Tokio runtime.
///
/// This is a native observation runner. A composition that deadlocks also
/// hangs here; deadlock handling belongs to the later modeled execution tier.
pub fn run_task_set_native(set: laplace_rt::TaskSet) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("laplace: tokio runtime build failed");
    let local = tokio::task::LocalSet::new();

    runtime.block_on(local.run_until(async move {
        let joins = set
            .into_tasks()
            .into_iter()
            .map(tokio::task::spawn_local)
            .collect::<Vec<_>>();

        for join in joins {
            let _ = join.await;
        }
    }));
}
