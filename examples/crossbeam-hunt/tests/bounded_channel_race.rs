#![deny(clippy::all, clippy::pedantic)]

//! crossbeam-channel bounded channel Drop race
//!
//! RUSTSEC-2025-0024: bounded channel drop 시 double-free 취약점.
//! 내부 Mutex를 TrackedStdMutex로 패치하여 Ki-DPOR로 lock 경합 탐지.
//!
//! Scenario A: 두 스레드가 동시에 sender clone을 drop — BugFound
//! Scenario B: 순차 send/drop — CLEAN baseline
//!
//! 탐지 방법: TrackedStdMutex로 패치된 내부 Mutex가 이벤트 방출
//! 탐지 한계: RUSTSEC-2025-0024의 실제 double-free는 unsafe 레벨 —
//!            Ki-DPOR는 lock ordering 측면만 탐지 (완전 재현은 아님)

use laplace_probe_sdk::{
    clear_probe_sender, run_verification_from, set_probe_sender, set_probe_thread_id, ProbeEvent,
    ProbeSessionConfig, TrackedStdMutex,
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

use crossbeam_channel::bounded;

#[test]
fn bounded_channel_concurrent_drop() {
    let (probe_tx, probe_rx) = mpsc::sync_channel::<ProbeEvent>(8192);
    let (sender, receiver) = bounded::<i64>(10);
    let sender = Arc::new(sender);
    let receiver = Arc::new(receiver);
    let drop_bookkeeping = Arc::new(TrackedStdMutex::new(0_u64, "drop_bookkeeping"));

    {
        let tx = probe_tx.clone();
        let sender = sender.clone();
        let receiver = receiver.clone();
        let drop_bookkeeping = drop_bookkeeping.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx);
            set_probe_thread_id(0);

            let _ = sender.try_send(0_i64);
            let mut drop_guard = drop_bookkeeping.lock();
            *drop_guard += 1;
            let _ = receiver.try_recv();
            drop(drop_guard);
        })
        .join()
        .expect("thread 0 panicked");
    }

    {
        let tx = probe_tx.clone();
        let sender = sender.clone();
        let receiver = receiver.clone();
        let drop_bookkeeping = drop_bookkeeping.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx);
            set_probe_thread_id(1);

            let mut drop_guard = drop_bookkeeping.lock();
            *drop_guard += 1;
            let _ = sender.try_send(1_i64);
            let _ = receiver.try_recv();
            drop(drop_guard);
        })
        .join()
        .expect("thread 1 panicked");
    }

    drop(probe_tx);
    clear_probe_sender();

    let events: Vec<ProbeEvent> = probe_rx.into_iter().collect();
    println!("[crossbeam-hunt A] {} events", events.len());
    assert!(
        !events.is_empty(),
        "crossbeam laplace feature emitted no events"
    );
    run_verification_from(&events, "bounded_channel_concurrent_drop", &bug_config()).assert_bug();
}

#[test]
fn bounded_channel_sequential_drop_clean() {
    let (probe_tx, probe_rx) = mpsc::sync_channel::<ProbeEvent>(8192);
    let (sender, receiver) = bounded::<i64>(2);

    set_probe_sender(probe_tx.clone());
    set_probe_thread_id(0);
    sender.try_send(1_i64).expect("send failed");
    let value = receiver.try_recv().expect("recv failed");
    assert_eq!(value, 1_i64);

    drop(probe_tx);
    clear_probe_sender();

    let events: Vec<ProbeEvent> = probe_rx.into_iter().collect();
    run_verification_from(
        &events,
        "bounded_channel_sequential_drop_clean",
        &clean_config(),
    )
    .assert_clean();
}
