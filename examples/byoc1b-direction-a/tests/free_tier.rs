#![deny(clippy::all, clippy::pedantic)]

use laplace_probe_sdk::{
    clear_probe_sender, install_probe_lock_hook, run_verification_from, set_probe_sender,
    set_probe_thread_id, ProbeEvent, ProbeSessionConfig, ReferenceVerdict,
};
use laplace_rt::{JoinToken, SpawnHook};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;

#[path = "../src/program.rs"]
mod program;

struct SequentialProbeSpawnHook {
    next_thread: AtomicU64,
}

impl SequentialProbeSpawnHook {
    const fn new() -> Self {
        Self {
            next_thread: AtomicU64::new(0),
        }
    }
}

impl SpawnHook for SequentialProbeSpawnHook {
    fn spawn(&self, f: Box<dyn FnOnce() + Send + 'static>) -> JoinToken {
        let thread = self.next_thread.fetch_add(1, Ordering::SeqCst);
        set_probe_thread_id(thread);
        f();
        JoinToken::engine()
    }
}

fn install_sequential_probe_spawn_hook() {
    laplace_rt::install_spawn_hook(std::sync::Arc::new(SequentialProbeSpawnHook::new()));
}

#[test]
fn shared_source_uses_real_parking_lot_mutex_without_laplace_mutex_traits() {
    let source = include_str!("../src/program.rs");

    assert!(source.contains("parking_lot::Mutex"));
    assert!(source.contains("std::thread::spawn"));
    assert!(source.contains("#[laplace::model]"));
    assert!(!source.contains("laplace_sync"));
    assert!(!source.contains("ModelLock"));
    assert!(!source.contains("env.spawn"));
    assert!(!source.contains("laplace_rt::spawn"));
}

#[test]
fn shared_source_uses_real_parking_lot_rwlock_without_laplace_mutex_traits() {
    let source = include_str!("../src/program.rs");

    assert!(source.contains("parking_lot::RwLock"));
    assert!(source.contains(".read()"));
    assert!(source.contains(".write()"));
    assert!(!source.contains("laplace_sync"));
    assert!(!source.contains("ModelLock"));
}

#[test]
fn rwlock_read_models_are_plain_parking_lot_bodies() {
    program::parking_lot_rwlock_read_read_ab_ba_program(|_thread, body| {
        body();
    });
    program::parking_lot_rwlock_read_write_ab_ba_program(|_thread, body| {
        body();
    });
}

#[test]
fn rwlock_fan_out_model_is_plain_parking_lot_body() {
    let spawned = std::cell::RefCell::new(Vec::new());

    program::parking_lot_rwlock_multi_reader_fan_out_program(|thread, _body| {
        spawned.borrow_mut().push(thread);
    });

    assert_eq!(program::FAN_OUT_RESOURCES, 2);
    assert_eq!(spawned.into_inner(), vec![0, 1, 2]);
}

#[test]
fn shared_source_uses_real_std_sync_mutex_without_laplace_mutex_traits() {
    let source = include_str!("../src/program.rs");

    assert!(source.contains("std::sync::Mutex"));
    assert!(source.contains("std::thread::spawn"));
    assert!(source.contains("#[laplace::model]"));
    assert!(!source.contains("laplace_rt::ModelMutex"));
    assert!(!source.contains("laplace_rt::spawn"));
    assert!(!source.contains("env.spawn"));
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
fn std_spawn_model_toy_route_detects_lock_order_cycle() {
    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(32);

    set_probe_sender(tx.clone());
    install_sequential_probe_spawn_hook();
    program::std_spawn_mutex_ab_ba_program();
    laplace_rt::clear_spawn_hook();
    clear_probe_sender();
    drop(tx);

    let events: Vec<ProbeEvent> = rx.into_iter().collect();
    assert!(events.len() >= program::AB_BA_RESOURCES * 2);

    let result = run_verification_from(
        &events,
        "byoc1b_direction_a_std_spawn_parking_lot_mutex",
        &ProbeSessionConfig::default(),
    );

    assert!(matches!(result.verdict, ReferenceVerdict::BugFound { .. }));
}

#[test]
fn std_sync_mutex_model_toy_route_detects_lock_order_cycle() {
    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(32);

    set_probe_sender(tx.clone());
    install_sequential_probe_spawn_hook();
    install_probe_lock_hook();
    program::std_sync_mutex_ab_ba_program();
    laplace_rt::clear_spawn_hook();
    laplace_rt::clear_lock_hook();
    clear_probe_sender();
    drop(tx);

    let events: Vec<ProbeEvent> = rx.into_iter().collect();
    assert!(events.len() >= program::AB_BA_RESOURCES * 2);

    let result = run_verification_from(
        &events,
        "byoc1b_direction_a_std_sync_mutex",
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
