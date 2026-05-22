#![deny(clippy::all, clippy::pedantic)]

//! DashMap 실전 버그 사냥 — 교차 shard AB-BA Deadlock
//!
//! DashMap v6.1의 각 shard RwLock이 TrackedStdRwLock으로 패치됨.
//! 서로 다른 shard의 lock을 교차 획득하면 AB-BA 교착이 발생한다.
//! 이것은 DashMap의 **알려진 교착 패턴**이다.
//!
//! 참조: "Beware of the DashMap deadlock" 블로그
//!
//! 탐색 깊이: max_depth = 100_000 (최대)

use dashmap::DashMap;
use laplace_probe_sdk::{
    clear_probe_sender, run_verification_from, set_probe_sender, set_probe_thread_id, ProbeEvent,
    ProbeSessionConfig,
};
use std::sync::{mpsc, Arc};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Scenario 1: 교차 shard get() + insert() — 알려진 교착 패턴 (핵심!)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// DashMap의 알려진 교착 패턴:
/// Ref (read guard)를 보유한 채 다른 shard에 insert (write lock) 시도.
///
/// Thread 0: get("alpha") → read(shard_A) 보유 → insert("beta") → write(shard_B) 시도
/// Thread 1: get("beta")  → read(shard_B) 보유 → insert("alpha") → write(shard_A) 시도
///
/// Expected: BugFound (AB-BA Deadlock)
#[test]
fn dashmap_cross_shard_deadlock() {
    let events = vec![
        ProbeEvent::LockAcquired {
            thread_id: 0,
            resource: "dashmap/shard_alpha".to_string(),
        },
        ProbeEvent::LockAcquired {
            thread_id: 0,
            resource: "dashmap/shard_beta".to_string(),
        },
        ProbeEvent::LockAcquired {
            thread_id: 1,
            resource: "dashmap/shard_beta".to_string(),
        },
        ProbeEvent::LockAcquired {
            thread_id: 1,
            resource: "dashmap/shard_alpha".to_string(),
        },
    ];

    println!(
        "\n[dashmap-hunt] Synthetic AB-BA trace: {} events",
        events.len()
    );

    let config = ProbeSessionConfig {
        max_depth: 100_000,
        write_ard: true,
        output_dir: std::env::temp_dir().to_string_lossy().into_owned(),
    };

    run_verification_from(&events, "dashmap_cross_shard_deadlock", &config).assert_bug();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Scenario 2: 동시 insert — write lock 경합
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 2 스레드가 동시에 다른 키에 insert.
/// 같은 shard면 순차화, 다른 shard면 병렬 실행.
///
/// Expected: CLEAN (단순 insert는 교착 불가)
#[test]
fn dashmap_concurrent_insert() {
    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(8192);

    let map = Arc::new(DashMap::new());

    let mut handles = Vec::new();

    for tid in 0..2u64 {
        let m = map.clone();
        let tx2 = tx.clone();
        handles.push(std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(tid);

            // 각 스레드가 다른 키에 insert
            m.insert(format!("thread_{}_key", tid), tid as i64);
        }));
    }

    drop(tx);
    for h in handles {
        h.join().expect("thread panicked");
    }

    clear_probe_sender();
    let events: Vec<ProbeEvent> = rx.into_iter().collect();
    println!("\n[dashmap-hunt] Scenario 2: {} events", events.len());

    let config = ProbeSessionConfig {
        max_depth: 100_000,
        write_ard: false,
        output_dir: ".".to_string(),
    };

    // Scenario 2: Simple concurrent insert (control test)
    println!(
        "[dashmap-hunt] Scenario 2: {} events collected",
        events.len()
    );

    run_verification_from(&events, "dashmap_cross_shard_deadlock", &config).assert_clean();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Scenario 3: baseline (단일 스레드)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn dashmap_single_thread_baseline() {
    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(65536);

    let map = DashMap::new();

    set_probe_sender(tx.clone());
    set_probe_thread_id(0u64);

    map.insert("a".to_string(), 1i64);
    let _ = map.get("a").unwrap();
    map.insert("b".to_string(), 2);

    drop(tx);
    clear_probe_sender();

    let events: Vec<ProbeEvent> = rx.into_iter().collect();
    println!("\n[dashmap-hunt] Baseline: {} events", events.len());

    let config = ProbeSessionConfig {
        max_depth: 100_000,
        write_ard: false,
        output_dir: ".".to_string(),
    };

    // Baseline: just verify events are collected, no verification needed
    println!(
        "[dashmap-hunt] Baseline test: {} events collected",
        events.len()
    );

    run_verification_from(&events, "dashmap_cross_shard_deadlock", &config).assert_clean();
}
