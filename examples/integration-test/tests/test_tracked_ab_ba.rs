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
        clear_probe_sender, run_verification_from, set_probe_sender, set_probe_thread_id,
        ProbeEvent, ProbeSessionConfig,
    };
    use std::sync::{mpsc, Arc};

    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(8192);

    // #[laplace_tracked]가 생성한 Default 사용
    let state = Arc::new(DualLockState::default());

    // [결정성 메모]: `TrackedMutex`는 실제 `tokio::sync::Mutex`를 감싼다. 두 스레드를
    // 동시에 돌리면 이 프로그램은 *진짜* AB-BA 교착에 빠질 수 있고(스레드 0이 a,b를
    // 모두 잡기 전에 스레드 1이 b를 잡으면 영구 블록), 캡처가 완료되지 못한다. 우리가
    // 필요한 것은 순서 관계를 담은 *트레이스*이지 실제 동시 실행이 아니므로, 두 락
    // 시퀀스를 순차로 실행한다 — 트레이스는 여전히 T0:a→b, T1:b→a 순환을 담아
    // 레퍼런스 검증기가 교착을 탐지한다. (실제 동시 인터리빙 탐색은 엔진의 몫이다.)
    let run_locker = |thread_id: u64, first_is_a: bool| {
        let s = state.clone();
        let tx2 = tx.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(thread_id);
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("rt");
            rt.block_on(async {
                if first_is_a {
                    // Thread 0: A → B
                    let _a = s.lock_a.lock().await;
                    let _b = s.lock_b.lock().await;
                } else {
                    // Thread 1: B → A (역순!)
                    let _b = s.lock_b.lock().await;
                    let _a = s.lock_a.lock().await;
                }
            });
        })
        .join()
        .expect("thread panicked");
    };

    run_locker(0, true);
    run_locker(1, false);

    // Legacy hand-written harness: the per-thread `set_probe_sender` also
    // registers a process-global sender clone. Clear it after both phases so the
    // last live clone drops and `rx` closes — otherwise `into_iter` hangs.
    drop(tx);
    clear_probe_sender();

    let events: Vec<ProbeEvent> = rx.into_iter().collect();

    let config = ProbeSessionConfig {
        write_ard: true,
        output_dir: std::env::temp_dir().to_string_lossy().into_owned(),
        ..ProbeSessionConfig::default()
    };

    // AB-BA → BugFound 기대
    run_verification_from(&events, "dual_lock_ab_ba", &config).assert_bug();
}
