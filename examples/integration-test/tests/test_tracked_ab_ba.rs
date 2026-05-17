//! Scenario 2: AB-BA 교착 — BugFound 기대
//!
//! 검증 항목:
//! - 2개 Mutex의 #[track] 자동 이름 지정 (lock_a, lock_b)
//! - #[laplace_sdk::verify(expected = "bug")]로 BugFound 검증
//! - Ki-DPOR가 AB-BA 순환을 탐지하는가

use laplace_sdk::prelude::*;

#[laplace_tracked]
struct DualLockState {
    #[track]
    lock_a: Mutex<u64>,
    #[track]
    lock_b: Mutex<u64>,
}

/// [설계 메모]: #[laplace_sdk::verify]는 모든 스레드가 동일 함수를 실행하므로
/// AB-BA를 구현하려면 수동 테스트를 작성해야 한다.
/// 이는 Loom도 동일한 제약이다.

// 수동 테스트 — AB-BA는 스레드별 다른 동작 필요
#[test]
fn test_dual_lock_ab_ba() {
    use laplace_sdk::{
        run_verification_from, set_probe_sender, set_probe_thread_id, ProbeEvent,
        ProbeSessionConfig,
    };
    use std::sync::{mpsc, Arc};

    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(8192);

    // #[laplace_tracked]가 생성한 Default 사용
    let state = Arc::new(DualLockState::default());

    let mut handles = Vec::new();

    // Thread 0: A → B
    {
        let s = state.clone();
        let tx2 = tx.clone();
        handles.push(std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(0u64);
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("rt");
            rt.block_on(async {
                let _a = s.lock_a.lock().await;
                let _b = s.lock_b.lock().await;
            });
        }));
    }

    // Thread 1: B → A (역순!)
    {
        let s = state.clone();
        let tx2 = tx.clone();
        handles.push(std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(1u64);
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("rt");
            rt.block_on(async {
                let _b = s.lock_b.lock().await;
                let _a = s.lock_a.lock().await;
            });
        }));
    }

    drop(tx);
    for h in handles {
        h.join().expect("thread panicked");
    }

    let events: Vec<ProbeEvent> = rx.into_iter().collect();

    let config = ProbeSessionConfig {
        write_ard: true,
        output_dir: std::env::temp_dir().to_string_lossy().into_owned(),
        ..ProbeSessionConfig::default()
    };

    // AB-BA → BugFound 기대
    run_verification_from(&events, "dual_lock_ab_ba", &config).assert_bug();
}
