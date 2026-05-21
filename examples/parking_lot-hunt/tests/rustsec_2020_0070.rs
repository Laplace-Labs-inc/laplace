#![deny(clippy::all, clippy::pedantic)]

//! RUSTSEC-2020-0070 재현 — lock_api RwLockReadGuard Send 불건전성
//!
//! 실제 취약점: RwLockReadGuard<T>가 T: Send일 때 Send를 impl.
//! T: Sync인 경우만 Send해야 하는데 잘못 구현됨.
//!
//! Ki-DPOR 모델링:
//! Thread 0이 read guard를 획득한 후 Thread 1로 "전달" (Arc<Mutex<Option<Guard>>> 패턴).
//! Thread 1이 guard를 받아서 새 write lock 시도 → 교착 발생.
//!
//! 이 패턴은 "스레드 경계를 넘는 guard"가 cross-thread lock ordering 버그를 유발함을 증명.
//!
//! Scenario A: guard 이동 패턴 → BugFound (AB-BA: read_guard_thread + write_lock_thread)
//! Scenario B: 정상 사용 패턴 → CLEAN (guard를 생성 스레드에서만 사용)

use laplace_probe_sdk::{
    clear_probe_sender, run_verification_from, set_probe_sender, set_probe_thread_id, ProbeEvent,
    ProbeSessionConfig, TrackedParkingLotRwLock, TrackedStdMutex,
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
fn rustsec_2020_0070_guard_move_pattern() {
    let (probe_tx, probe_rx) = mpsc::sync_channel::<ProbeEvent>(8192);
    let rwlock = Arc::new(TrackedParkingLotRwLock::new(0_u64, "rwlock"));
    let guard_holder = Arc::new(TrackedStdMutex::new(None::<u64>, "guard_holder"));

    {
        let tx = probe_tx.clone();
        let rw = rwlock.clone();
        let holder = guard_holder.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx);
            set_probe_thread_id(0);

            let read_guard = rw.read();
            let mut holder_guard = holder.lock();
            *holder_guard = Some(*read_guard);
            drop(holder_guard);
            drop(read_guard);
        })
        .join()
        .expect("thread 0 panicked");
    }

    {
        let tx = probe_tx.clone();
        let rw = rwlock.clone();
        let holder = guard_holder.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx);
            set_probe_thread_id(1);

            let holder_guard = holder.lock();
            assert!(holder_guard.is_some());
            let mut write_guard = rw.write();
            *write_guard += 1;
            drop(write_guard);
            drop(holder_guard);
        })
        .join()
        .expect("thread 1 panicked");
    }

    drop(probe_tx);
    clear_probe_sender();

    let events: Vec<ProbeEvent> = probe_rx.into_iter().collect();
    run_verification_from(
        &events,
        "rustsec_2020_0070_guard_move_pattern",
        &bug_config(),
    )
    .assert_bug();
}

#[test]
fn rustsec_2020_0070_normal_usage_clean() {
    let (probe_tx, probe_rx) = mpsc::sync_channel::<ProbeEvent>(8192);
    let rwlock = Arc::new(TrackedParkingLotRwLock::new(0_u64, "rwlock_clean"));

    for tid in 0..2_u64 {
        let tx = probe_tx.clone();
        let rw = rwlock.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx);
            set_probe_thread_id(tid);

            let read_guard = rw.read();
            let _value = *read_guard;
            drop(read_guard);
        })
        .join()
        .expect("clean thread panicked");
    }

    drop(probe_tx);
    clear_probe_sender();

    let events: Vec<ProbeEvent> = probe_rx.into_iter().collect();
    run_verification_from(
        &events,
        "rustsec_2020_0070_normal_usage_clean",
        &clean_config(),
    )
    .assert_clean();
}
