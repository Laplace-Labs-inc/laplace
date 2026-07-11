// SPDX-License-Identifier: Apache-2.0

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

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
    let completed = events
        .iter()
        .filter(|event| matches!(event, laplace_sdk::ProbeEvent::TaskCompleted { .. }))
        .count();
    let polled = events
        .iter()
        .filter(|event| matches!(event, laplace_sdk::ProbeEvent::TaskPolled { .. }))
        .count();
    let ready = events
        .iter()
        .filter(|event| matches!(event, laplace_sdk::ProbeEvent::FutureReady { .. }))
        .count();
    assert_eq!(spawned, 3);
    assert_eq!(completed, 3);
    assert!(polled >= 3);
    assert!(ready >= 3);

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
        3
    );
    assert_eq!(
        dumped
            .iter()
            .filter(|event| matches!(event, laplace_sdk::ProbeEvent::TaskCompleted { .. }))
            .count(),
        3
    );
    assert!(dumped
        .iter()
        .any(|event| matches!(event, laplace_sdk::ProbeEvent::TaskPolled { .. })));

    let _ = std::fs::remove_dir_all(&dir);
}
