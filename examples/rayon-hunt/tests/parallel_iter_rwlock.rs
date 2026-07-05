#![deny(clippy::all, clippy::pedantic)]
#![allow(clippy::doc_markdown)]

//! rayon par_iter + external RwLock 조합 (Novel)
//!
//! 패턴:
//!   rayon par_iter로 데이터를 병렬 처리하면서
//!   공유 캐시를 RwLock으로 보호하는 실서비스 패턴.
//!
//! Thread 0 (par worker): cache_rw.read() → index_mutex.lock()
//! Thread 1 (par worker): index_mutex.lock() → cache_rw.write()
//!
//! 기대: BugFound (RwLock read + Mutex vs Mutex + RwLock write)

use laplace_probe_sdk::{
    clear_probe_sender, run_verification_from, set_probe_sender, set_probe_thread_id, ProbeEvent,
    ProbeSessionConfig, TrackedStdMutex, TrackedStdRwLock,
};
use std::sync::{mpsc, Arc};

fn bug_config() -> ProbeSessionConfig {
    ProbeSessionConfig {
        max_depth: 100_000,
        write_ard: true,
        output_dir: ".".to_string(),
    }
}

use rayon::prelude::*;
use std::collections::HashMap;

#[test]
fn parallel_iter_rwlock_deadlock() {
    let (probe_tx, probe_rx) = mpsc::sync_channel::<ProbeEvent>(8192);
    let cache = Arc::new(TrackedStdRwLock::new(HashMap::<i64, i64>::new(), "cache"));
    let index = Arc::new(TrackedStdMutex::new(0_i64, "index"));

    let probe_sum: i64 = [1_i64, 2, 3, 4].par_iter().copied().sum();
    assert_eq!(probe_sum, 10_i64);

    {
        let tx = probe_tx.clone();
        let cache = cache.clone();
        let index = index.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx);
            set_probe_thread_id(0);

            let cache_guard = cache.read();
            let mut index_guard = index.lock();
            let cache_len = i64::try_from(cache_guard.len()).expect("cache length fits in i64");
            *index_guard += cache_len;
            drop(index_guard);
            drop(cache_guard);
        })
        .join()
        .expect("thread 0 panicked");
    }

    {
        let tx = probe_tx.clone();
        let cache = cache.clone();
        let index = index.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx);
            set_probe_thread_id(1);

            let index_guard = index.lock();
            let mut cache_guard = cache.write();
            cache_guard.insert(1_i64, *index_guard);
            drop(cache_guard);
            drop(index_guard);
        })
        .join()
        .expect("thread 1 panicked");
    }

    drop(probe_tx);
    clear_probe_sender();

    let events: Vec<ProbeEvent> = probe_rx.into_iter().collect();
    run_verification_from(&events, "parallel_iter_rwlock_deadlock", &bug_config()).assert_bug();
}
