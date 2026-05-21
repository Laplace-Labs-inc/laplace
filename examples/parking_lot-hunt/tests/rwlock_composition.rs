#![deny(clippy::all, clippy::pedantic)]

//! parking_lot RwLock + std::sync::Mutex 조합 교착 (Novel)
//!
//! 실서비스 패턴:
//!   RwLock = 캐시 (자주 읽기, 드물게 쓰기)
//!   Mutex  = DB 연결 풀 (독점 접근)
//!
//! Thread 0 (캐시 조회 후 DB 갱신):
//!   cache_rwlock.read() [캐시 miss 확인] → conn_pool_mutex.lock() [DB 조회]
//!
//! Thread 1 (DB 결과로 캐시 갱신):
//!   conn_pool_mutex.lock() [DB 완료] → cache_rwlock.write() [캐시 업데이트]
//!
//! 기대: BugFound (실서비스에서 자주 발생하는 패턴)

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

use std::collections::HashMap;

#[test]
fn parking_lot_rwlock_mutex_composition_deadlock() {
    let (probe_tx, probe_rx) = mpsc::sync_channel::<ProbeEvent>(8192);
    let cache = Arc::new(TrackedParkingLotRwLock::new(
        HashMap::<String, i64>::new(),
        "cache",
    ));
    let conn_pool = Arc::new(TrackedStdMutex::new(Vec::<i64>::new(), "conn_pool"));

    {
        let tx = probe_tx.clone();
        let cache = cache.clone();
        let conn_pool = conn_pool.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx);
            set_probe_thread_id(0);

            let cache_guard = cache.read();
            let _miss = cache_guard.get("key");
            let mut pool_guard = conn_pool.lock();
            pool_guard.push(1);
            drop(pool_guard);
            drop(cache_guard);
        })
        .join()
        .expect("thread 0 panicked");
    }

    {
        let tx = probe_tx.clone();
        let cache = cache.clone();
        let conn_pool = conn_pool.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx);
            set_probe_thread_id(1);

            let pool_guard = conn_pool.lock();
            let mut cache_guard = cache.write();
            cache_guard.insert("key".to_string(), pool_guard.len() as i64);
            drop(cache_guard);
            drop(pool_guard);
        })
        .join()
        .expect("thread 1 panicked");
    }

    drop(probe_tx);
    clear_probe_sender();

    let events: Vec<ProbeEvent> = probe_rx.into_iter().collect();
    run_verification_from(
        &events,
        "parking_lot_rwlock_mutex_composition",
        &bug_config(),
    )
    .assert_bug();
}
