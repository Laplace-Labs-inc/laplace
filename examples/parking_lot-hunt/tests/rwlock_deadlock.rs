#![deny(clippy::all, clippy::pedantic)]

//! parking_lot RwLock 교착 패턴
//!
//! Scenario A: 두 RwLock 교차 획득 → BugFound
//!   Thread 0: rwlock_a.read() → rwlock_b.write()
//!   Thread 1: rwlock_b.read() → rwlock_a.write()
//!
//! Scenario B: 단일 RwLock read → 동일 스레드 write 업그레이드 시도
//!   (parking_lot은 RwLock 업그레이드 미지원 — deadlock)
//!
//! Scenario C: CLEAN baseline — 동일 순서로 모든 스레드가 획득

use laplace_probe_sdk::{
    clear_probe_sender, emit, run_verification_from, set_probe_sender, set_probe_thread_id,
    ProbeEvent, ProbeSessionConfig, TrackedParkingLotRwLock, TrackedStdMutex,
};
use std::sync::{mpsc, Arc};

fn bug_config() -> ProbeSessionConfig {
    ProbeSessionConfig {
        max_depth: 100_000,
        write_ard: true,
        output_dir: ".".to_string(),
    }
}

fn clean_config() -> ProbeSessionConfig {
    ProbeSessionConfig {
        max_depth: 100_000,
        write_ard: false,
        output_dir: ".".to_string(),
    }
}

#[test]
fn rwlock_cross_acquisition_deadlock() {
    let (probe_tx, probe_rx) = mpsc::sync_channel::<ProbeEvent>(8192);
    let rw_a = Arc::new(TrackedParkingLotRwLock::new(0_u64, "rw_a"));
    let rw_b = Arc::new(TrackedParkingLotRwLock::new(0_u64, "rw_b"));

    {
        let tx = probe_tx.clone();
        let a = rw_a.clone();
        let b = rw_b.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx);
            set_probe_thread_id(0);

            let read_a = a.read();
            let mut write_b = b.write();
            *write_b = *read_a + 1;
            drop(write_b);
            drop(read_a);
        })
        .join()
        .expect("thread 0 panicked");
    }

    {
        let tx = probe_tx.clone();
        let a = rw_a.clone();
        let b = rw_b.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx);
            set_probe_thread_id(1);

            let read_b = b.read();
            let mut write_a = a.write();
            *write_a = *read_b + 1;
            drop(write_a);
            drop(read_b);
        })
        .join()
        .expect("thread 1 panicked");
    }

    drop(probe_tx);
    clear_probe_sender();

    let events: Vec<ProbeEvent> = probe_rx.into_iter().collect();
    run_verification_from(
        &events,
        "parking_lot_rwlock_cross_acquisition",
        &bug_config(),
    )
    .assert_bug();
}

#[test]
fn rwlock_read_to_write_upgrade_deadlock() {
    let (probe_tx, probe_rx) = mpsc::sync_channel::<ProbeEvent>(8192);
    let rw = Arc::new(TrackedParkingLotRwLock::new(0_u64, "rw_upgrade"));
    let upgrade_gate = Arc::new(TrackedStdMutex::new((), "upgrade_gate"));

    {
        let tx = probe_tx.clone();
        let rw = rw.clone();
        let gate = upgrade_gate.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx);
            set_probe_thread_id(0);

            let read_guard = rw.read();
            let gate_guard = gate.lock();
            emit(ProbeEvent::RwLockWriteAcquired {
                thread_id: 0,
                resource: "rw_upgrade".to_string(),
            });
            emit(ProbeEvent::RwLockWriteReleased {
                thread_id: 0,
                resource: "rw_upgrade".to_string(),
            });
            drop(gate_guard);
            drop(read_guard);
        })
        .join()
        .expect("thread 0 panicked");
    }

    {
        let tx = probe_tx.clone();
        let rw = rw.clone();
        let gate = upgrade_gate.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx);
            set_probe_thread_id(1);

            let gate_guard = gate.lock();
            let mut write_guard = rw.write();
            *write_guard += 1;
            drop(write_guard);
            drop(gate_guard);
        })
        .join()
        .expect("thread 1 panicked");
    }

    drop(probe_tx);
    clear_probe_sender();

    let events: Vec<ProbeEvent> = probe_rx.into_iter().collect();
    run_verification_from(
        &events,
        "parking_lot_rwlock_upgrade_deadlock",
        &bug_config(),
    )
    .assert_bug();
}

#[test]
fn rwlock_same_order_baseline_clean() {
    let (probe_tx, probe_rx) = mpsc::sync_channel::<ProbeEvent>(8192);
    let rw_a = Arc::new(TrackedParkingLotRwLock::new(0_u64, "rw_clean_a"));
    let rw_b = Arc::new(TrackedParkingLotRwLock::new(0_u64, "rw_clean_b"));

    for tid in 0..2_u64 {
        let tx = probe_tx.clone();
        let a = rw_a.clone();
        let b = rw_b.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx);
            set_probe_thread_id(tid);

            let read_a = a.read();
            let read_b = b.read();
            let _sum = *read_a + *read_b;
            drop(read_b);
            drop(read_a);
        })
        .join()
        .expect("clean thread panicked");
    }

    drop(probe_tx);
    clear_probe_sender();

    let events: Vec<ProbeEvent> = probe_rx.into_iter().collect();
    run_verification_from(
        &events,
        "parking_lot_rwlock_same_order_clean",
        &clean_config(),
    )
    .assert_clean();
}
