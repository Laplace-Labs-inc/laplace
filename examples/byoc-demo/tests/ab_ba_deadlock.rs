#![deny(clippy::all, clippy::pedantic)]
#![allow(clippy::doc_markdown)]

//! AB-BA 교착 패턴 탐지 증명.
//!
//! Thread 0: lock_a → lock_b
//! Thread 1: lock_b → lock_a  ← 역순!
//!
//! 실제로 두 스레드가 역순으로 락을 잡게 하면 테스트 자체가 교착될 수 있다.
//! 이 테스트는 관측된 이벤트 trace를 직접 구성하여
//! "Thread 0이 lock_a를 잡은 채 lock_b를 기다리는 동안
//!  Thread 1이 lock_b를 잡고 lock_a를 기다리는" 시나리오를 발견한다.
//!
//! 기대 결과: BugFound (교착 탐지)

use laplace_probe_sdk::{run_verification_from, ProbeEvent, ProbeSessionConfig};

// ── 테스트 ────────────────────────────────────────────────────────────────────

#[test]
fn ab_ba_deadlock_must_be_detected() {
    let events = vec![
        ProbeEvent::LockAcquired {
            thread_id: 0,
            resource: "lock_a".to_string(),
        },
        ProbeEvent::LockAcquired {
            thread_id: 0,
            resource: "lock_b".to_string(),
        },
        ProbeEvent::LockAcquired {
            thread_id: 1,
            resource: "lock_b".to_string(),
        },
        ProbeEvent::LockAcquired {
            thread_id: 1,
            resource: "lock_a".to_string(),
        },
    ];

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
