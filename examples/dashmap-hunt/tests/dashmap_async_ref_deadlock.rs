#![deny(clippy::all, clippy::pedantic)]

//! DashMap async Ref deadlock — Ki-DPOR 탐지
//!
//! 알려진 패턴: GitHub Issues #79, #253
//! Ref (read guard) 보유 중 다른 shard에 대한 insert (write lock) 시도.
//! async 코드에서는 .await 포인트에서 Ref를 보유하면 더 쉽게 발생.
//!
//! 이미 cross_shard 테스트에서 검증됨 — 이 테스트는 "Ref 보유 기간이 긴" 패턴
//! (실제 async 코드의 .await 지점을 시뮬레이션)에 집중한다.
//!
//! Scenario A: Ref + RefMut 교차 획득 (더 긴 hold time 시뮬레이션) — BugFound
//! Scenario B: 동일 shard 연속 접근 (CLEAN baseline) — CLEAN

use dashmap::DashMap;
use laplace_probe_sdk::{
    clear_probe_sender, run_verification_from, set_probe_sender, set_probe_thread_id, ProbeEvent,
    ProbeSessionConfig,
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

fn different_shard_keys(map: &DashMap<String, i64>) -> (String, String) {
    let first = "alpha_0".to_string();
    let first_shard = map.determine_map(first.as_str());
    for idx in 1..10_000 {
        let candidate = format!("beta_{idx}");
        if map.determine_map(candidate.as_str()) != first_shard {
            return (first, candidate);
        }
    }
    panic!("failed to find keys on different shards");
}

fn same_shard_keys(map: &DashMap<String, i64>) -> (String, String) {
    let first = "same_0".to_string();
    let first_shard = map.determine_map(first.as_str());
    for idx in 1..10_000 {
        let candidate = format!("same_{idx}");
        if map.determine_map(candidate.as_str()) == first_shard {
            return (first, candidate);
        }
    }
    panic!("failed to find keys on the same shard");
}

#[test]
fn dashmap_async_ref_deadlock() {
    let (probe_tx, probe_rx) = mpsc::sync_channel::<ProbeEvent>(8192);
    let map = Arc::new(DashMap::with_shard_amount(4));
    let (alpha, beta) = different_shard_keys(&map);
    map.insert(alpha.clone(), 1_i64);
    map.insert(beta.clone(), 2_i64);

    {
        let tx = probe_tx.clone();
        let map = map.clone();
        let alpha = alpha.clone();
        let beta = beta.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx);
            set_probe_thread_id(0);

            let ref_alpha = map.get(&alpha).expect("alpha key missing");
            let _value = *ref_alpha;
            map.insert(beta, 10);
            drop(ref_alpha);
        })
        .join()
        .expect("thread 0 panicked");
    }

    {
        let tx = probe_tx.clone();
        let map = map.clone();
        let alpha = alpha.clone();
        let beta = beta.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx);
            set_probe_thread_id(1);

            let ref_beta = map.get(&beta).expect("beta key missing");
            let _value = *ref_beta;
            map.insert(alpha, 20);
            drop(ref_beta);
        })
        .join()
        .expect("thread 1 panicked");
    }

    drop(probe_tx);
    clear_probe_sender();

    let events: Vec<ProbeEvent> = probe_rx.into_iter().collect();
    println!("[dashmap-async-ref] {} events", events.len());
    run_verification_from(&events, "dashmap_async_ref_deadlock", &bug_config()).assert_bug();
}

#[test]
fn dashmap_same_shard_ref_baseline_clean() {
    let (probe_tx, probe_rx) = mpsc::sync_channel::<ProbeEvent>(8192);
    let map = Arc::new(DashMap::with_shard_amount(4));
    let (alpha, beta) = same_shard_keys(&map);
    map.insert(alpha.clone(), 1_i64);
    map.insert(beta.clone(), 2_i64);

    let tx = probe_tx.clone();
    let map = map.clone();
    std::thread::spawn(move || {
        set_probe_sender(tx);
        set_probe_thread_id(0);

        let ref_alpha = map.get(&alpha).expect("alpha key missing");
        let _value = *ref_alpha;
        drop(ref_alpha);
        map.insert(beta, 3_i64);
    })
    .join()
    .expect("baseline thread panicked");

    drop(probe_tx);
    clear_probe_sender();

    let events: Vec<ProbeEvent> = probe_rx.into_iter().collect();
    println!("[dashmap-async-ref-clean] {} events", events.len());
    run_verification_from(
        &events,
        "dashmap_same_shard_ref_baseline_clean",
        &clean_config(),
    )
    .assert_clean();
}
