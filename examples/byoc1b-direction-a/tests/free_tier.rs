#![deny(clippy::all, clippy::pedantic)]

use laplace_probe_sdk::{
    clear_probe_sender, run_verification_from, set_probe_sender, set_probe_thread_id, ProbeEvent,
    ProbeSessionConfig, ReferenceVerdict,
};
use std::sync::mpsc;

#[path = "../src/program.rs"]
mod program;

#[test]
fn shared_source_uses_real_parking_lot_mutex_without_laplace_mutex_traits() {
    let source = include_str!("../src/program.rs");

    assert!(source.contains("parking_lot::Mutex"));
    assert!(!source.contains("laplace_sync"));
    assert!(!source.contains("ModelLock"));
}

#[test]
fn shared_source_uses_real_parking_lot_rwlock_without_laplace_mutex_traits() {
    let source = include_str!("../src/program.rs");

    assert!(source.contains("parking_lot::RwLock"));
    assert!(!source.contains("laplace_sync"));
    assert!(!source.contains("ModelLock"));
}

#[test]
fn same_source_toy_route_detects_lock_order_cycle() {
    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(32);

    program::parking_lot_mutex_ab_ba_program(|thread, body| {
        set_probe_sender(tx.clone());
        set_probe_thread_id(thread as u64);
        body();
        clear_probe_sender();
    });
    drop(tx);

    let events: Vec<ProbeEvent> = rx.into_iter().collect();
    assert!(events.len() >= program::AB_BA_RESOURCES * 2);

    let result = run_verification_from(
        &events,
        "byoc1b_direction_a_parking_lot_mutex",
        &ProbeSessionConfig::default(),
    );

    assert!(matches!(result.verdict, ReferenceVerdict::BugFound { .. }));
}

#[test]
fn same_source_rwlock_toy_route_detects_lock_order_cycle() {
    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(32);

    program::parking_lot_rwlock_ab_ba_program(|thread, body| {
        set_probe_sender(tx.clone());
        set_probe_thread_id(thread as u64);
        body();
        clear_probe_sender();
    });
    drop(tx);

    let events: Vec<ProbeEvent> = rx.into_iter().collect();
    assert!(events.len() >= program::AB_BA_RESOURCES * 2);

    let result = run_verification_from(
        &events,
        "byoc1b_direction_a_parking_lot_rwlock",
        &ProbeSessionConfig::default(),
    );

    assert!(matches!(result.verdict, ReferenceVerdict::BugFound { .. }));
}
