// SPDX-License-Identifier: Apache-2.0

use std::future::poll_fn;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::Poll;

#[laplace_sdk::verify(tasks, name = "task_set_e2e", expected = "clean")]
fn task_set_e2e(tasks: &mut laplace_sdk::rt::TaskSet) {
    let notify = Arc::new(laplace_sdk::rt::ModelAsyncNotify::new());
    let producer_lock = Arc::new(laplace_sdk::rt::ModelMutex::new(0_u8));
    let waiter_lock = Arc::new(laplace_sdk::rt::ModelMutex::new(0_u8));

    let producer_notify = Arc::clone(&notify);
    let producer_lock_for_task = Arc::clone(&producer_lock);
    tasks.spawn(async move {
        let _guard = producer_lock_for_task.lock().expect("producer lock");
        producer_notify.notify_one();
        tokio::spawn(async {});
    });

    let waiter_notify = Arc::clone(&notify);
    let waiter_lock_for_task = Arc::clone(&waiter_lock);
    let waiter = tasks.spawn(async move {
        waiter_notify.notified().await;
        let _guard = waiter_lock_for_task.lock().expect("waiter lock");
    });

    tasks.spawn(async move {
        waiter.await;
    });
}

static TEST_GUARD: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    TEST_GUARD.lock().unwrap_or_else(PoisonError::into_inner)
}

#[test]
fn task_set_native_run_emits_and_dumps_task_events() {
    let _serial = serial();
    let dir = std::env::temp_dir().join(format!(
        "laplace-task-set-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create event directory");
    std::env::set_var("LAPLACE_VERIFY_EVENTS_DIR", &dir);

    let session = laplace_sdk::CaptureSession::begin();
    laplace_probe_sdk::install_probe_lock_hook();
    laplace_probe_sdk::install_probe_task_hook();
    laplace_sdk::set_probe_thread_id(0);
    let mut tasks = laplace_sdk::rt::TaskSet::new();
    task_set_e2e(&mut tasks);
    laplace_probe_sdk::run_task_set_native(tasks);
    let events = session.finish();
    laplace_sdk::dump_events_if_configured(
        "task_set_manual",
        "clean",
        "fully_deterministic",
        &events,
    );

    std::env::remove_var("LAPLACE_VERIFY_EVENTS_DIR");

    let spawned = events
        .iter()
        .filter(|event| matches!(event, laplace_sdk::ProbeEvent::TaskSpawned { .. }))
        .count();
    let mut completed_registered: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            laplace_sdk::ProbeEvent::TaskCompleted { task_id } if *task_id < (1_u64 << 63) => {
                Some(*task_id)
            }
            _ => None,
        })
        .collect();
    completed_registered.sort_unstable();
    let polled = events
        .iter()
        .filter(|event| matches!(event, laplace_sdk::ProbeEvent::TaskPolled { .. }))
        .count();
    let ready = events
        .iter()
        .filter(|event| matches!(event, laplace_sdk::ProbeEvent::FutureReady { .. }))
        .count();
    assert_eq!(spawned, 4);
    assert_eq!(completed_registered, vec![0, 1, 2]);
    assert!(polled >= 3);
    assert!(ready >= 3);
    assert!(events.iter().any(|event| matches!(
        event,
        laplace_sdk::ProbeEvent::TaskSpawned {
            task_id,
            parent_task_id: Some(0),
            source_location: None,
        } if *task_id >= (1_u64 << 63)
    )));

    let lock_threads: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            laplace_sdk::ProbeEvent::LockAcquired { thread_id, .. } => Some(*thread_id),
            _ => None,
        })
        .collect();
    assert_eq!(lock_threads.len(), 2);
    assert!(lock_threads.contains(&0));
    assert!(lock_threads.contains(&1));

    let path = dir.join("task_set_manual.json");
    let envelope: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("task event envelope written"))
            .expect("valid task event envelope");
    let dumped: Vec<laplace_sdk::ProbeEvent> =
        serde_json::from_value(envelope["events"].clone()).expect("events round-trip");
    assert_eq!(
        dumped
            .iter()
            .filter(|event| matches!(event, laplace_sdk::ProbeEvent::TaskSpawned { .. }))
            .count(),
        4
    );
    let mut dumped_completed_registered: Vec<_> = dumped
        .iter()
        .filter_map(|event| match event {
            laplace_sdk::ProbeEvent::TaskCompleted { task_id } if *task_id < (1_u64 << 63) => {
                Some(*task_id)
            }
            _ => None,
        })
        .collect();
    dumped_completed_registered.sort_unstable();
    assert_eq!(dumped_completed_registered, vec![0, 1, 2]);
    assert!(dumped
        .iter()
        .any(|event| matches!(event, laplace_sdk::ProbeEvent::TaskPolled { .. })));

    let _ = std::fs::remove_dir_all(&dir);
}

fn yield_once() -> impl std::future::Future<Output = ()> {
    let mut yielded = false;
    poll_fn(move |cx| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    })
}

fn capture_async_task_events(
    build: impl FnOnce(&mut laplace_sdk::rt::TaskSet),
) -> Vec<laplace_sdk::ProbeEvent> {
    let session = laplace_sdk::CaptureSession::begin();
    laplace_probe_sdk::install_probe_task_hook();
    laplace_probe_sdk::install_probe_async_hooks();
    laplace_sdk::set_probe_thread_id(0);

    let mut tasks = laplace_sdk::rt::TaskSet::new();
    build(&mut tasks);
    laplace_probe_sdk::run_task_set_native(tasks);
    let events = session.finish();

    laplace_sdk::rt::clear_async_channel_hook();
    laplace_sdk::rt::clear_async_lock_hook();
    laplace_sdk::rt::clear_async_notify_hook();
    laplace_sdk::rt::clear_task_observer_hook();
    events
}

#[test]
fn async_lock_capture_keeps_task_thread_ownership() {
    let _serial = serial();
    let lock = Arc::new(laplace_sdk::rt::ModelAsyncMutex::new(0_u8));
    let first_lock = Arc::clone(&lock);
    let second_lock = Arc::clone(&lock);

    let events = capture_async_task_events(move |tasks| {
        tasks.spawn(async move {
            let _guard = first_lock.lock().await;
            yield_once().await;
        });
        tasks.spawn(async move {
            let _guard = second_lock.lock().await;
        });
    });

    let acquired: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            laplace_sdk::ProbeEvent::AsyncLockAcquired { thread_id, .. } => Some(*thread_id),
            _ => None,
        })
        .collect();
    let released: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            laplace_sdk::ProbeEvent::AsyncLockReleased { thread_id, .. } => Some(*thread_id),
            _ => None,
        })
        .collect();

    assert_eq!(acquired.len(), 2);
    assert_eq!(released.len(), 2);
    assert!(acquired.contains(&0));
    assert!(acquired.contains(&1));
    assert!(released.contains(&0));
    assert!(released.contains(&1));
}

#[test]
fn native_fire_and_forget_spawn_emits_dynamic_task_marker() {
    let _serial = serial();
    let events = capture_async_task_events(|tasks| {
        tasks.spawn(async {
            laplace_sdk::rt::spawn_task(async {});
        });
    });

    let dynamic_ids: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            laplace_sdk::ProbeEvent::TaskSpawned {
                task_id,
                parent_task_id: Some(0),
                source_location: None,
            } if *task_id >= (1_u64 << 63) => Some(*task_id),
            _ => None,
        })
        .collect();
    assert_eq!(
        dynamic_ids.len(),
        1,
        "native dynamic spawn marker missing: {events:?}"
    );
}

#[test]
fn dynamic_spawn_capture_emits_two_parent_attributed_envelopes() {
    let _serial = serial();
    let first = capture_async_task_events(task_set_e2e);
    let second = capture_async_task_events(task_set_e2e);

    for events in [&first, &second] {
        assert!(events.iter().any(|event| matches!(
            event,
            laplace_sdk::ProbeEvent::TaskSpawned {
                task_id,
                parent_task_id: Some(0),
                source_location: None,
            } if *task_id >= (1_u64 << 63)
        )));
    }

    laplace_sdk::dump_events_if_configured(
        "dynamic_spawn_capture_one",
        "clean",
        "fully_deterministic",
        &first,
    );
    laplace_sdk::dump_events_if_configured(
        "dynamic_spawn_capture_two",
        "clean",
        "fully_deterministic",
        &second,
    );
}

#[test]
fn async_channel_capture_reports_kind_and_successful_operations() {
    let _serial = serial();
    let events = capture_async_task_events(|tasks| {
        let (sender, mut receiver) = laplace_sdk::rt::mpsc::channel::<u8>(1);
        tasks.spawn(async move {
            sender.send(7).await.expect("send succeeds");
        });
        tasks.spawn(async move {
            assert_eq!(receiver.recv().await, Some(7));
        });
    });

    assert!(events.iter().any(|event| matches!(
        event,
        laplace_sdk::ProbeEvent::AsyncChannelCreated {
            thread_id: 0,
            kind: laplace_sdk::AsyncChannelKind::MpscBounded { capacity: 1 },
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        laplace_sdk::ProbeEvent::AsyncChannelOpResolved {
            op_kind: laplace_sdk::AsyncChannelOp::Send,
            outcome: laplace_sdk::AsyncChannelOutcome::Ok,
            ..
        }
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        laplace_sdk::ProbeEvent::AsyncChannelOpResolved {
            op_kind: laplace_sdk::AsyncChannelOp::Recv,
            outcome: laplace_sdk::AsyncChannelOutcome::Ok,
            ..
        }
    )));
}

#[test]
fn async_notify_capture_reports_one_and_wait_resolution() {
    let _serial = serial();
    let notify = Arc::new(laplace_sdk::rt::ModelAsyncNotify::new());
    let waiter_notify = Arc::clone(&notify);
    let notifier_notify = Arc::clone(&notify);

    let events = capture_async_task_events(move |tasks| {
        tasks.spawn(async move {
            waiter_notify.notified().await;
        });
        tasks.spawn(async move {
            notifier_notify.notify_one();
        });
    });

    assert!(events
        .iter()
        .any(|event| matches!(event, laplace_sdk::ProbeEvent::AsyncNotifyOne { .. })));
    assert!(events.iter().any(|event| matches!(
        event,
        laplace_sdk::ProbeEvent::AsyncNotifyWaitResolved { .. }
    )));
}

#[test]
fn async_event_envelope_round_trips_all_new_variants_and_legacy_events() {
    let _serial = serial();
    let events = vec![
        laplace_sdk::ProbeEvent::AsyncLockRequested {
            thread_id: 0,
            resource: 1,
            waiter: 2,
            kind: laplace_sdk::AsyncAcquireKind::Mutex,
        },
        laplace_sdk::ProbeEvent::AsyncLockAcquired {
            thread_id: 1,
            resource: 1,
            waiter: 2,
            kind: laplace_sdk::AsyncAcquireKind::RwRead,
        },
        laplace_sdk::ProbeEvent::AsyncLockReleased {
            thread_id: 1,
            resource: 1,
            waiter: 2,
            kind: laplace_sdk::AsyncAcquireKind::RwWrite,
        },
        laplace_sdk::ProbeEvent::AsyncLockWaiterDropped {
            thread_id: 1,
            resource: 1,
            waiter: 3,
        },
        laplace_sdk::ProbeEvent::AsyncSemaphoreCreated {
            thread_id: 0,
            resource: 4,
            permits: 3,
        },
        laplace_sdk::ProbeEvent::AsyncPermitsAdded {
            thread_id: 0,
            resource: 4,
            n: 2,
        },
        laplace_sdk::ProbeEvent::AsyncNotifyWaitRequested {
            thread_id: 0,
            resource: 5,
            waiter: 6,
        },
        laplace_sdk::ProbeEvent::AsyncNotifyWaitResolved {
            thread_id: 1,
            resource: 5,
            waiter: 6,
        },
        laplace_sdk::ProbeEvent::AsyncNotifyOne {
            thread_id: 1,
            resource: 5,
        },
        laplace_sdk::ProbeEvent::AsyncNotifyWaiters {
            thread_id: 1,
            resource: 5,
        },
        laplace_sdk::ProbeEvent::AsyncNotifyWaiterDropped {
            thread_id: 0,
            resource: 5,
            waiter: 7,
        },
        laplace_sdk::ProbeEvent::AsyncChannelCreated {
            thread_id: 0,
            channel: 8,
            kind: laplace_sdk::AsyncChannelKind::MpscBounded { capacity: 1 },
        },
        laplace_sdk::ProbeEvent::AsyncChannelOpRequested {
            thread_id: 0,
            channel: 8,
            op: 9,
            op_kind: laplace_sdk::AsyncChannelOp::Send,
        },
        laplace_sdk::ProbeEvent::AsyncChannelOpResolved {
            thread_id: 1,
            channel: 8,
            op: 9,
            op_kind: laplace_sdk::AsyncChannelOp::Recv,
            outcome: laplace_sdk::AsyncChannelOutcome::Ok,
        },
        laplace_sdk::ProbeEvent::AsyncChannelOpDropped {
            thread_id: 0,
            channel: 8,
            op: 10,
        },
        laplace_sdk::ProbeEvent::AsyncChannelEndpointCloned {
            thread_id: 0,
            channel: 8,
            side: laplace_sdk::AsyncChannelSide::Sender,
        },
        laplace_sdk::ProbeEvent::AsyncChannelEndpointDropped {
            thread_id: 1,
            channel: 8,
            side: laplace_sdk::AsyncChannelSide::Receiver,
        },
        laplace_sdk::ProbeEvent::AsyncChannelClosed {
            thread_id: 1,
            channel: 8,
        },
    ];

    let dir = std::env::temp_dir().join(format!(
        "laplace-d8-2-events-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    ));
    std::env::set_var("LAPLACE_VERIFY_EVENTS_DIR", &dir);
    laplace_sdk::dump_events_if_configured(
        "d8_2_async_event_schema",
        "clean",
        "fully_deterministic",
        &events,
    );
    std::env::remove_var("LAPLACE_VERIFY_EVENTS_DIR");

    let envelope: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("d8_2_async_event_schema.json"))
            .expect("async event envelope written"),
    )
    .expect("valid async event envelope");
    assert_eq!(envelope["schema_version"], "2");
    let round_tripped: Vec<laplace_sdk::ProbeEvent> =
        serde_json::from_value(envelope["events"].clone()).expect("events round-trip");
    assert_eq!(
        serde_json::to_value(&round_tripped).expect("serialize round-trip"),
        serde_json::to_value(&events).expect("serialize source")
    );

    let legacy: serde_json::Value = serde_json::from_str(
        r#"{"target":"legacy","expected":"clean","events":[{"LockAcquired":{"thread_id":0,"resource":"legacy"}}]}"#,
    )
    .expect("legacy envelope parses");
    let legacy_events: Vec<laplace_sdk::ProbeEvent> =
        serde_json::from_value(legacy["events"].clone()).expect("legacy events parse");
    assert!(matches!(
        legacy_events.as_slice(),
        [laplace_sdk::ProbeEvent::LockAcquired { .. }]
    ));

    let _ = std::fs::remove_dir_all(&dir);
}
