#![deny(clippy::all, clippy::pedantic)]

//! AB-BA 교착 패턴 탐지 증명.
//!
//! Thread 0: lock_a → lock_b
//! Thread 1: lock_b → lock_a  ← 역순!
//!
//! 실제 실행에서는 교착이 발생하지 않는다 (두 스레드가 완료됨).
//! 그러나 Ki-DPOR는 모든 가능한 인터리빙을 탐색하여
//! "Thread 0이 lock_a를 잡은 채 lock_b를 기다리는 동안
//!  Thread 1이 lock_b를 잡고 lock_a를 기다리는" 시나리오를 발견한다.
//!
//! 기대 결과: BugFound (교착 탐지)

use laplace_probe_sdk::{
    run_verification_from, set_probe_sender, set_probe_thread_id, ProbeEvent, ProbeSessionConfig,
    TrackedMutex,
};
use std::sync::{mpsc, Arc};

// ── 공유 상태 — 두 개의 독립 락 ─────────────────────────────────────────────

struct TwoLockState {
    lock_a: TrackedMutex<i64>,
    lock_b: TrackedMutex<i64>,
}

impl Default for TwoLockState {
    fn default() -> Self {
        Self {
            lock_a: TrackedMutex::new(0, "lock_a"),
            lock_b: TrackedMutex::new(0, "lock_b"),
        }
    }
}

// ── 테스트 ────────────────────────────────────────────────────────────────────

#[test]
fn ab_ba_deadlock_must_be_detected() {
    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(4096);
    let state = Arc::new(TwoLockState::default());
    let mut handles = Vec::new();

    // ── Thread 0: lock_a → lock_b ─────────────────────────────────────────────
    {
        let s = state.clone();
        let tx2 = tx.clone();
        handles.push(std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(0);
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("rt build failed");
            rt.block_on(async {
                let _a = s.lock_a.lock().await; // Request(lock_a)
                let _b = s.lock_b.lock().await; // Request(lock_b)
                                                // _b dropped → Release(lock_b)
                                                // _a dropped → Release(lock_a)
            });
        }));
    }

    // ── Thread 1: lock_b → lock_a (역순) ──────────────────────────────────────
    {
        let s = state.clone();
        let tx2 = tx.clone();
        handles.push(std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(1);
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("rt build failed");
            rt.block_on(async {
                let _b = s.lock_b.lock().await; // Request(lock_b)
                let _a = s.lock_a.lock().await; // Request(lock_a)
                                                // _a dropped → Release(lock_a)
                                                // _b dropped → Release(lock_b)
            });
        }));
    }

    // ── 수집 ─────────────────────────────────────────────────────────────────
    drop(tx); // 주 채널 종료
    for h in handles {
        h.join().expect("verification thread panicked");
    }
    let events: Vec<ProbeEvent> = rx.into_iter().collect();

    println!("\n[AB-BA Demo] Collected {} events:", events.len());
    for (i, e) in events.iter().enumerate() {
        println!("  [{i}] {e:?}");
    }

    // ── Ki-DPOR 실행 ─────────────────────────────────────────────────────────
    let config = ProbeSessionConfig {
        write_ard: true,
        output_dir: std::env::temp_dir().to_string_lossy().into_owned(),
        ..ProbeSessionConfig::default()
    };

    // AB-BA 교착 → BugFound 기대
    run_verification_from(&events, "ab_ba_deadlock_demo", &config).assert_bug();
}
