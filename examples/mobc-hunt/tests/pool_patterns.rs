#![deny(clippy::all, clippy::pedantic)]
#![allow(clippy::doc_markdown)]
// legacy 표면 커버리지용, 이관은 후속; attribute macro lint는 item-level allow보다 먼저 발생한다.
#![allow(deprecated)]

//! mobc connection pool 패턴 검증.
//!
//! Scenario A: pool_get_put       — 단일 락, CLEAN 기대 (mobc 자체 패턴)
//! Scenario B: pool_multi_thread  — 단일 락 × 4스레드, CLEAN 기대
//! Scenario C: pool_healthcheck   — 이중 락 역순, BugFound 기대 (naïve extension 버그)

use laplace_macro::axiom_target;
use laplace_probe_sdk::{
    clear_probe_sender, run_verification_from, set_probe_sender, set_probe_thread_id, ProbeEvent,
    ProbeSessionConfig, TrackedMutex,
};
use std::collections::{HashMap, VecDeque};
use std::sync::{mpsc, Arc};

// ═══════════════════════════════════════════════════════════════════════════════
// Scenario A: mobc Pool 단일 락 패턴 — CLEAN baseline
// ═══════════════════════════════════════════════════════════════════════════════
//
// mobc SharedPool 재현:
//   SharedPool { internals: Mutex<PoolInternals> }
//   PoolInternals { conns: VecDeque<i64>, num_conns: u32 }
//
// Pool::get() 패턴:
//   1. lock internals
//   2. pop idle conn (있으면 반환)
//   3. drop lock (없으면 connect 후 reacquire — 여기서는 단순화)
//   4. return conn
//
// Pool::put_back() 패턴:
//   1. lock internals
//   2. push conn
//   3. drop lock

struct PoolInternals {
    conns: VecDeque<i64>,
}

struct MobcPool {
    internals: TrackedMutex<PoolInternals>,
}

impl Default for MobcPool {
    fn default() -> Self {
        let internals = PoolInternals {
            conns: VecDeque::from(vec![1, 2, 3, 4]),
        };
        Self {
            internals: TrackedMutex::new(internals, "pool_internals"),
        }
    }
}

/// Scenario A: mobc get() → put_back() 단일 락 패턴.
/// 단일 락이므로 AB-BA 불가 → CLEAN.
///
/// 각 스레드가 동일한 코드를 실행하면 DPOR가 같은 인터리빙을 탐색하므로
/// 단순화: 단일 lock/unlock 쌍만 수행.
// legacy 표면 커버리지용, 이관은 후속
#[allow(deprecated)]
#[axiom_target(threads = 2, name = "pool_get_put")]
async fn pool_get_put(pool: Arc<MobcPool>) {
    // 단일 락 획득/해제 (모든 스레드가 동일)
    let _conn = {
        let mut internals = pool.internals.lock().await;
        internals.conns.pop_front().unwrap_or(0)
    };
}

// ═══════════════════════════════════════════════════════════════════════════════
// Scenario B: 4-thread Pool 동시 접근 — CLEAN baseline
// ═══════════════════════════════════════════════════════════════════════════════

/// Scenario B: 4개 스레드가 동시에 get/put_back. 단일 락.
/// 락 하나만 존재하므로 어떤 인터리빙도 교착 불가 → CLEAN.
// legacy 표면 커버리지용, 이관은 후속
#[allow(deprecated)]
#[axiom_target(threads = 4, name = "pool_concurrent")]
async fn pool_concurrent(pool: Arc<MobcPool>) {
    {
        let mut internals = pool.internals.lock().await;
        let conn = internals.conns.pop_front().unwrap_or(99);
        internals.conns.push_back(conn);
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Scenario C: Pool + HealthCheck 이중 락 — BugFound 기대
// ═══════════════════════════════════════════════════════════════════════════════
//
// 실제 extension 패턴:
//   - Pool::get(): pool_state → conn_meta 순서로 락 획득
//   - HealthChecker::run(): conn_meta → pool_state 순서로 락 획득 (역순!)
//
// 이것이 현실의 connection pool extension에서 흔히 발생하는 AB-BA.
// 예: health check 스레드가 연결 상태 메타데이터를 먼저 잠그고,
//     죽은 연결을 pool에서 제거하기 위해 pool 락을 추가로 잡는 패턴.

struct PoolWithMeta {
    pool_state: TrackedMutex<Vec<i64>>,          // Lock A
    conn_meta: TrackedMutex<HashMap<i64, bool>>, // Lock B
}

impl Default for PoolWithMeta {
    fn default() -> Self {
        let mut meta = HashMap::new();
        meta.insert(1, true);
        meta.insert(2, true);
        Self {
            pool_state: TrackedMutex::new(vec![1, 2], "pool_state"),
            conn_meta: TrackedMutex::new(meta, "conn_meta"),
        }
    }
}

/// Scenario C: AB-BA 이중 락 — BugFound 기대.
///
/// 이 테스트는 `#[axiom_target]`을 쓰지 않고 SDK를 직접 사용한다.
/// Thread 0 (Pool::get 패턴): pool_state → conn_meta
/// Thread 1 (HealthChecker 패턴): conn_meta → pool_state
#[test]
fn pool_healthcheck_ab_ba_must_be_detected() {
    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(4096);
    let pool = Arc::new(PoolWithMeta::default());
    let mut handles = Vec::new();

    // ── Thread 0: Pool::get() — pool_state → conn_meta ────────────────────────
    {
        let p = pool.clone();
        let tx2 = tx.clone();
        handles.push(std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(0);
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("rt");
            rt.block_on(async {
                // Pool::get(): 먼저 pool_state 잠금 (Lock A)
                let _state = p.pool_state.lock().await;
                // 연결 메타데이터 확인을 위해 conn_meta 잠금 (Lock B)
                let _meta = p.conn_meta.lock().await;
                // _meta dropped → Release B
                // _state dropped → Release A
            });
        }));
    }

    // ── Thread 1: HealthChecker — conn_meta → pool_state (역순!) ──────────────
    {
        let p = pool.clone();
        let tx2 = tx.clone();
        handles.push(std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(1);
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("rt");
            rt.block_on(async {
                // HealthChecker: 먼저 conn_meta 잠금 (Lock B)
                let _meta = p.conn_meta.lock().await;
                // 죽은 연결 제거를 위해 pool_state 잠금 (Lock A) — 역순!
                let _state = p.pool_state.lock().await;
                // _state dropped → Release A
                // _meta dropped → Release B
            });
        }));
    }

    drop(tx);
    for h in handles {
        h.join().expect("thread panicked");
    }
    // set_probe_sender는 전역 슬롯에도 클론을 남기므로 수집 전에 클리어해야
    // rx.into_iter()가 종료된다.
    clear_probe_sender();
    let events: Vec<ProbeEvent> = rx.into_iter().collect();

    println!(
        "\n[mobc HealthCheck AB-BA] Collected {} events:",
        events.len()
    );
    for (i, e) in events.iter().enumerate() {
        println!("  [{i}] {e:?}");
    }

    let config = ProbeSessionConfig {
        write_ard: true,
        output_dir: std::env::temp_dir().to_string_lossy().into_owned(),
        ..ProbeSessionConfig::default()
    };

    // Pool + HealthChecker 이중 락 역순 → BugFound 기대
    run_verification_from(&events, "pool_healthcheck_ab_ba", &config).assert_bug();
}
