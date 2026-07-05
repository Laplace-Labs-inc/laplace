#![deny(clippy::all, clippy::pedantic)]
#![allow(clippy::doc_markdown)]

//! rayon 중첩 scope + external Mutex 교착 (GitHub Issue #1174 패턴)
//!
//! Ki-DPOR 탐지 범위: rayon 자체 내부가 아닌 client code TrackedMutex.
//!
//! 패턴 설명:
//!   rayon::spawn + 외부 Mutex 보유 중 추가 rayon 작업 대기 시,
//!   thread pool 작업자들이 서로를 기다리는 교착.
//!
//! Ki-DPOR 모델링:
//!   실제 rayon pool 고갈은 재현 어려움 → 동등한 lock ordering 패턴으로 모델링.
//!
//! Thread 0 (rayon worker 역할):
//!   coordinator.lock() [작업 등록] → result.lock() [결과 대기]
//!
//! Thread 1 (rayon worker 역할):
//!   result.lock() [결과 준비] → coordinator.lock() [완료 알림]
//!
//! 기대: BugFound

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

#[test]
fn nested_scope_deadlock() {
    let (probe_tx, probe_rx) = mpsc::sync_channel::<ProbeEvent>(8192);
    let coordinator = Arc::new(TrackedStdMutex::new(0_u64, "coordinator"));
    let result = Arc::new(TrackedStdMutex::new(0_u64, "result"));

    rayon::scope(|scope| {
        scope.spawn(|_| {});
    });

    {
        let tx = probe_tx.clone();
        let coordinator = coordinator.clone();
        let result = result.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx);
            set_probe_thread_id(0);

            let mut coordinator_guard = coordinator.lock();
            *coordinator_guard += 1;
            let mut result_guard = result.lock();
            *result_guard += 1;
            drop(result_guard);
            drop(coordinator_guard);
        })
        .join()
        .expect("thread 0 panicked");
    }

    {
        let tx = probe_tx.clone();
        let coordinator = coordinator.clone();
        let result = result.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx);
            set_probe_thread_id(1);

            let mut result_guard = result.lock();
            *result_guard += 1;
            let mut coordinator_guard = coordinator.lock();
            *coordinator_guard += 1;
            drop(coordinator_guard);
            drop(result_guard);
        })
        .join()
        .expect("thread 1 panicked");
    }

    drop(probe_tx);
    clear_probe_sender();

    let events: Vec<ProbeEvent> = probe_rx.into_iter().collect();
    run_verification_from(&events, "nested_scope_deadlock", &bug_config()).assert_bug();
}

#[test]
fn rayon_scope_with_tracked_mutex() {
    let (probe_tx, probe_rx) = mpsc::sync_channel::<ProbeEvent>(8192);
    let shared = Arc::new(TrackedStdMutex::new(vec![0_i64; 4], "shared_vec"));

    rayon::scope(|scope| {
        scope.spawn(|_| {});
    });

    for tid in 0..2_u64 {
        let tx = probe_tx.clone();
        let shared = shared.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx);
            set_probe_thread_id(tid);

            let mut guard = shared.lock();
            let index = usize::try_from(tid).expect("thread id fits in usize");
            let value = i64::try_from(tid).expect("thread id fits in i64");
            guard[index] = value;
            drop(guard);
        })
        .join()
        .expect("clean thread panicked");
    }

    drop(probe_tx);
    clear_probe_sender();

    let events: Vec<ProbeEvent> = probe_rx.into_iter().collect();
    run_verification_from(&events, "rayon_shared_mutex_clean", &clean_config()).assert_clean();
}
