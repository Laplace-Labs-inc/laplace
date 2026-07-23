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

impl laplace_model_rt::TaskObserverHook for ProbeTaskHook {
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

    fn poll_completed(&self, task: u64, attempt: u64, outcome: laplace_model_rt::TaskPollOutcome) {
        match outcome {
            laplace_model_rt::TaskPollOutcome::Pending => {
                emit(ProbeEvent::FuturePending {
                    task_id: task,
                    future_id: None,
                    poll_attempt_id: attempt,
                });
            }
            laplace_model_rt::TaskPollOutcome::Ready => {
                emit(ProbeEvent::FutureReady {
                    task_id: task,
                    future_id: None,
                    poll_attempt_id: attempt,
                });
            }
            laplace_model_rt::TaskPollOutcome::Panicked => {}
        }
        clear_current_task_id();
    }

    fn task_completed(&self, task: u64) {
        clear_current_task_id();
        emit(ProbeEvent::TaskCompleted { task_id: task });
    }

    fn join_requested(&self, joined: u64) {
        let Some(thread_id) = current_task_id() else {
            return;
        };
        emit(ProbeEvent::TaskJoinRequested {
            thread_id,
            joined_task_id: joined,
        });
    }

    fn join_resolved(&self, joined: u64) {
        let Some(thread_id) = current_task_id() else {
            return;
        };
        emit(ProbeEvent::TaskJoinResolved {
            thread_id,
            joined_task_id: joined,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::ProbeEvent;
    use crate::session::{clear_probe_sender, set_probe_sender};
    use laplace_model_rt::TaskObserverHook;
    use std::sync::mpsc;

    #[test]
    fn join_attribution_requires_a_current_task() {
        let (tx, rx) = mpsc::sync_channel(8);
        set_probe_sender(tx);
        clear_current_task_id();
        assert_eq!(current_task_id(), None);
        let hook = ProbeTaskHook;
        hook.join_requested(9);
        hook.join_resolved(9);
        clear_probe_sender();
        let events: Vec<ProbeEvent> = rx.into_iter().collect();
        assert!(events.is_empty(), "task-less joins must not be captured");

        let (tx, rx) = mpsc::sync_channel(8);
        set_probe_sender(tx);
        set_current_task_id(7);
        assert_eq!(current_task_id(), Some(7));
        hook.join_requested(9);
        clear_probe_sender();
        let events: Vec<ProbeEvent> = rx.into_iter().collect();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events.as_slice(),
            [ProbeEvent::TaskJoinRequested {
                thread_id: 7,
                joined_task_id: 9,
            }]
        ));

        clear_current_task_id();
    }
}

/// Installs the process-local probe task hook.
pub fn install_probe_task_hook() {
    laplace_model_rt::install_task_observer_hook(Arc::new(ProbeTaskHook));
}

/// Runs all registered tasks once on a current-thread Tokio runtime.
///
/// This is a native observation runner. A composition that deadlocks also
/// hangs here; deadlock handling belongs to the later modeled execution tier.
pub fn run_task_set_native(set: laplace_model_rt::TaskSet) {
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
