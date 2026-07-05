//! bb8-hunt Phase 4-B Revision: dual-lock AB-BA 패턴
//!
//! 목표: TrackedStdMutex 2개로 AB-BA 교착 경로를 구성하여 Ki-DPOR가 BugFound를 반환하는지 검증.
//!
//! 자원:
//!   - "pool_state"   — bb8 Pool 내부 상태를 나타내는 외부 TrackedStdMutex
//!   - "health_check" — 헬스 체크 메타데이터를 나타내는 외부 TrackedStdMutex
//!
//! Thread 0 (Pool::get 경로): pool_state → health_check
//! Thread 1 (HealthChecker): health_check → pool_state  ← 역순!
//!
//! 예상 결과: BugFound (AB-BA cycle)
//!
//! 주의: 두 락을 중첩(nested)으로 잡을 필요는 없다.
//!       Ki-DPOR는 이벤트 인터리빙을 전수 탐색하므로 순차 획득(sequential)도 탐지한다.
//!       Phase 4-C known-bug-hunt의 Thread 0도 순차였으나 BugFound 확인됨.

use laplace_probe_sdk::{
    run_verification_from, set_probe_sender, set_probe_thread_id, ProbeEvent, ProbeSessionConfig,
    TrackedStdMutex,
};
use std::sync::{mpsc, Arc};

/// 두 자원에 대한 AB-BA 잠금 순서를 Ki-DPOR로 검증한다.
#[test]
fn bb8_dual_lock_ab_ba() {
    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(4096);

    // ── 두 자원 ──────────────────────────────────────────────────────────────
    // pool_state  = Lock A  (bb8 Pool 내부 상태 역할)
    // health_check = Lock B  (헬스 체크 메타데이터 역할)
    let pool_state = Arc::new(TrackedStdMutex::new(0u64, "pool_state"));
    let health_check = Arc::new(TrackedStdMutex::new(0u64, "health_check"));

    let mut handles = Vec::new();

    // ── Thread 0: A → B 순서, 중첩 잠금 (Pool::get 경로) ──────────────────────
    {
        let ps = pool_state.clone();
        let hc = health_check.clone();
        let tx2 = tx.clone();
        handles.push(std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(0u64);

            // Lock A (pool_state) 획득 중에 Lock B (health_check) 획득 시도
            let _ga = ps.lock();
            // ─ Lock A 보유 중 ─
            let _gb = hc.lock();
            // ─ Lock A, B 모두 보유 ─
            // 두 락 모두 해제됨 (Drop)
        }));
    }

    // ── Thread 1: B → A 역순, 중첩 잠금 (HealthChecker 경로) ────────────────────
    {
        let ps = pool_state.clone();
        let hc = health_check.clone();
        let tx2 = tx.clone();
        handles.push(std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(1u64);

            // Lock B (health_check) 획득 중에 Lock A (pool_state) 획득 시도 ← 역순!
            let _gb = hc.lock();
            // ─ Lock B 보유 중 ─
            let _ga = ps.lock();
            // ─ Lock B, A 모두 보유 ─
            // 두 락 모두 해제됨 (Drop)
        }));
    }

    drop(tx);
    for h in handles {
        h.join().expect("thread panicked");
    }

    let events: Vec<ProbeEvent> = rx.into_iter().collect();

    println!("\n[bb8-hunt] Collected {} events:", events.len());
    for (i, e) in events.iter().enumerate() {
        println!("  [{i}] {e:?}");
    }

    let resource_names: std::collections::HashSet<_> = events
        .iter()
        .filter_map(|e| match e {
            ProbeEvent::LockAcquired { resource, .. } => Some(resource.as_str()),
            _ => None,
        })
        .collect();
    println!(
        "관측 자원 수 (= TrackedStdMutex 수): {}",
        resource_names.len()
    );
    println!("자원: {resource_names:?}");

    // 자원 2개 필수 확인
    assert_eq!(
        resource_names.len(),
        2,
        "자원이 2개여야 AB-BA 탐지 가능. 현재: {}개",
        resource_names.len()
    );

    let config = ProbeSessionConfig {
        write_ard: true,
        output_dir: std::env::temp_dir().to_string_lossy().into_owned(),
        ..ProbeSessionConfig::default()
    };

    // AB-BA → BugFound 기대
    run_verification_from(&events, "bb8_dual_lock_ab_ba", &config).assert_bug();
}
