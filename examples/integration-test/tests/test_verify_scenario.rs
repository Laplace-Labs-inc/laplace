//! Scenario mode: native one-shot capture with divergent worker bodies.
//!
//! These tests cover the `#[laplace_sdk::verify(scenario)]` path, where the
//! annotated body is the whole scenario instead of one replica worker body.

use std::collections::BTreeSet;
use std::sync::Arc;

use laplace_sdk::ProbeEvent;

#[laplace_sdk::verify(scenario, expected = "clean")]
fn scenario_clean_same_lock_order() {
    let lock_a = Arc::new(std::sync::Mutex::new(0_u64));
    let lock_b = Arc::new(std::sync::Mutex::new(0_u64));

    let worker_one_a = Arc::clone(&lock_a);
    let worker_one_b = Arc::clone(&lock_b);
    let worker_one = std::thread::spawn(move || {
        let mut a = worker_one_a.lock().unwrap();
        let mut b = worker_one_b.lock().unwrap();
        *a += 1;
        *b += 1;
    });

    let worker_two_a = Arc::clone(&lock_a);
    let worker_two_b = Arc::clone(&lock_b);
    let worker_two = std::thread::spawn(move || {
        let mut a = worker_two_a.lock().unwrap();
        let mut b = worker_two_b.lock().unwrap();
        *a += 1;
        *b += 1;
    });

    worker_one.join().unwrap();
    worker_two.join().unwrap();
}

#[test]
fn scenario_capture_assigns_implicit_ids_to_divergent_workers() {
    use laplace_sdk::__macro_support::{install_probe_lock_hook, CaptureSession};

    let session = CaptureSession::begin();
    install_probe_lock_hook();

    let lock_a = Arc::new(laplace_sdk::rt::ModelMutex::new(0_u64));
    let lock_b = Arc::new(laplace_sdk::rt::ModelMutex::new(0_u64));

    let worker_one_a = Arc::clone(&lock_a);
    let worker_one_b = Arc::clone(&lock_b);
    std::thread::spawn(move || {
        let mut a = worker_one_a.lock().unwrap();
        let mut b = worker_one_b.lock().unwrap();
        *a += 1;
        *b += 1;
    })
    .join()
    .unwrap();

    let worker_two_a = Arc::clone(&lock_a);
    let worker_two_b = Arc::clone(&lock_b);
    std::thread::spawn(move || {
        let mut b = worker_two_b.lock().unwrap();
        let mut a = worker_two_a.lock().unwrap();
        *b += 1;
        *a += 1;
    })
    .join()
    .unwrap();

    let events = session.finish();
    laplace_sdk::rt::clear_lock_hook();

    let thread_ids = events
        .iter()
        .filter_map(|event| match event {
            ProbeEvent::LockAcquired { thread_id, .. }
            | ProbeEvent::LockReleased { thread_id, .. } => Some(*thread_id),
            _ => None,
        })
        .collect::<BTreeSet<_>>();

    assert_eq!(
        thread_ids.len(),
        2,
        "expected two implicit child thread ids"
    );
}
