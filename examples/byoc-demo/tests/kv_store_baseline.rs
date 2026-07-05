#![deny(clippy::all, clippy::pedantic)]
#![allow(clippy::doc_markdown)]

//! KvStore CLEAN Baseline.
//!
//! 단일 TrackedMutex 기반 키-값 저장소.
//! 동시 읽기/쓰기가 있지만 락이 하나뿐이라 AB-BA 교착 불가.
//!
//! 기대 결과: Clean (false-positive 없음 확인)

use laplace_macro::axiom_target;
use laplace_probe_sdk::TrackedMutex;
use std::collections::HashMap;
use std::sync::Arc;

// ── KvStore — mini-redis Db와 동일한 패턴 (단일 Mutex<State>) ────────────────

struct KvStore {
    data: TrackedMutex<HashMap<String, String>>,
}

impl Default for KvStore {
    fn default() -> Self {
        Self {
            data: TrackedMutex::new(HashMap::new(), "kv_data"),
        }
    }
}

// ── 검증 대상 함수 — 단순 단일락 순차 접근 ────────────────────────────────────

#[axiom_target(threads = 2, name = "kv_single_lock")]
async fn kv_single_lock(store: Arc<KvStore>) {
    // 단일 lock 획득 및 해제
    {
        let mut map = store.data.lock().await;
        map.insert("key".to_string(), "val".to_string());
    } // 락 해제
}

// ── 3-thread 쓰기 집중 시나리오 ──────────────────────────────────────────────

struct Counter {
    value: TrackedMutex<i64>,
}

impl Default for Counter {
    fn default() -> Self {
        Self {
            value: TrackedMutex::new(0, "counter"),
        }
    }
}

#[axiom_target(threads = 3, name = "counter_increment")]
async fn counter_increment(state: Arc<Counter>) {
    let mut val = state.value.lock().await;
    *val += 1;
    // 락 해제 (val drop)
}
// 기대: Clean (단일 락, 순서 위반 불가)
