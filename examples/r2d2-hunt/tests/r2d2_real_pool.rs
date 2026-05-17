#![deny(clippy::all, clippy::pedantic)]

//! r2d2 실전 버그 사냥 — Ki-DPOR 최대 깊이 탐색
//!
//! r2d2 v0.8.10의 내부 `parking_lot::Mutex<PoolInternals>`가
//! `TrackedStdMutex`로 패치됨.
//! 동기식 pool이므로 blocking lock 경합이 핵심 사냥 대상.
//!
//! 탐색 깊이: max_depth = 100_000 (최대)

use laplace_probe_sdk::TrackedStdMutex;
use r2d2::{ManageConnection, Pool};
use std::sync::Arc;

// ── MockManager — 동기식 즉시 반환 커넥션 관리자 ──────────────────────────────

#[derive(Debug)]
struct MockManager;

impl ManageConnection for MockManager {
    type Connection = i64;
    type Error = std::io::Error;

    fn connect(&self) -> Result<Self::Connection, Self::Error> {
        Ok(42) // 즉시 반환
    }

    fn is_valid(&self, _conn: &mut Self::Connection) -> Result<(), Self::Error> {
        Ok(())
    }

    fn has_broken(&self, _conn: &mut Self::Connection) -> bool {
        false
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Scenario 1: 기본 pool.get() 동시 호출 (2 스레드, 동기)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

struct R2d2PoolState {
    pool: Pool<MockManager>,
}

impl Default for R2d2PoolState {
    fn default() -> Self {
        let pool = Pool::builder()
            .max_size(4)
            .min_idle(Some(2)) // 시작 시 2개 idle
            .test_on_check_out(false) // health check 비활성화
            .build(MockManager)
            .unwrap();
        Self { pool }
    }
}

/// 2 스레드 동기 pool.get() 경합.
/// r2d2 내부 Mutex lock이 TrackedStdMutex로 추적됨.
///
/// max_depth = 100_000
#[test]
fn r2d2_pool_get_2thread() {
    let (tx, rx) = std::sync::mpsc::sync_channel(8192);
    let state = Arc::new(R2d2PoolState::default());
    let mut handles = Vec::new();

    for thread_id in 0..2 {
        let s = state.clone();
        let tx2 = tx.clone();
        handles.push(std::thread::spawn(move || {
            laplace_probe_sdk::set_probe_sender(tx2);
            laplace_probe_sdk::set_probe_thread_id(thread_id as u64);

            let conn = s.pool.get().unwrap();
            let _ = *conn;
        }));
    }

    drop(tx);
    for h in handles {
        h.join().expect("thread panicked");
    }

    let events: Vec<_> = rx.into_iter().collect();
    let config = laplace_probe_sdk::ProbeSessionConfig {
        max_depth: 100_000,
        write_ard: true,
        output_dir: std::env::temp_dir().to_string_lossy().into_owned(),
    };

    laplace_probe_sdk::run_verification_from(&events, "r2d2_pool_get_2thread", &config);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Scenario 2: 3 스레드 동시 checkout (max_size=4)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 3 스레드가 동시에 동기 pool.get() 호출.
/// max_size=4이므로 모두 성공해야 하나, 내부 lock 경합 발생.
///
/// max_depth = 100_000
#[test]
fn r2d2_pool_get_3thread() {
    let (tx, rx) = std::sync::mpsc::sync_channel(8192);
    let state = Arc::new(R2d2PoolState::default());
    let mut handles = Vec::new();

    for thread_id in 0..3 {
        let s = state.clone();
        let tx2 = tx.clone();
        handles.push(std::thread::spawn(move || {
            laplace_probe_sdk::set_probe_sender(tx2);
            laplace_probe_sdk::set_probe_thread_id(thread_id as u64);

            let conn = s.pool.get().unwrap();
            let _ = *conn;
        }));
    }

    drop(tx);
    for h in handles {
        h.join().expect("thread panicked");
    }

    let events: Vec<_> = rx.into_iter().collect();
    let config = laplace_probe_sdk::ProbeSessionConfig {
        max_depth: 100_000,
        write_ard: true,
        output_dir: std::env::temp_dir().to_string_lossy().into_owned(),
    };

    laplace_probe_sdk::run_verification_from(&events, "r2d2_pool_get_3thread", &config);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Scenario 3: 외부 lock + pool.get() AB-BA 패턴 (핵심!)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 이것이 r2d2의 핵심 사냥 시나리오.
///
/// 실전 패턴: 사용자 코드가 자신의 Mutex를 잡은 상태에서 pool.get()을 호출.
/// pool.get()은 내부적으로 r2d2의 Mutex를 잡는다.
/// 다른 스레드가 역순으로 접근하면 AB-BA 교착.
///
/// Thread 0: external_lock → pool.get() (내부 r2d2_lock)  → A→B
/// Thread 1: pool.get() (내부 r2d2_lock) → external_lock  → B→A
///
/// 풀을 사전에 충분히 채워서 Condvar.wait()에 진입하지 않도록 한다.
/// (TrackedStdMutex는 parking_lot::Condvar와 호환되지 않으므로)
///
/// max_depth = 100_000

struct AbBaState {
    pool: Pool<MockManager>,
    external_lock: TrackedStdMutex<u64>,
}

impl Default for AbBaState {
    fn default() -> Self {
        let pool = Pool::builder()
            .max_size(8) // 충분히 커서 대기 없이 즉시 반환
            .min_idle(Some(4))
            .test_on_check_out(false)
            .build(MockManager)
            .unwrap();
        Self {
            pool,
            external_lock: TrackedStdMutex::new(0u64, "user_state"),
        }
    }
}

/// AB-BA 교착 탐색:
/// 모든 스레드가 external_lock → pool.get() 순서로 접근하되,
/// Ki-DPOR가 인터리빙을 조작하여 역순 경로를 탐색한다.
///
/// 실제로는 두 스레드가 동일 코드를 실행하지만,
/// lock 획득 시점의 인터리빙에 따라 교차 의존이 발생할 수 있다.
///
/// max_depth = 100_000
#[test]
fn r2d2_external_lock_get() {
    let (tx, rx) = std::sync::mpsc::sync_channel(8192);
    let state = Arc::new(AbBaState::default());
    let mut handles = Vec::new();

    for thread_id in 0..2 {
        let s = state.clone();
        let tx2 = tx.clone();
        handles.push(std::thread::spawn(move || {
            laplace_probe_sdk::set_probe_sender(tx2);
            laplace_probe_sdk::set_probe_thread_id(thread_id as u64);

            let _guard = s.external_lock.lock();
            let conn = s.pool.get().unwrap();
            let _ = *conn;
        }));
    }

    drop(tx);
    for h in handles {
        h.join().expect("thread panicked");
    }

    let events: Vec<_> = rx.into_iter().collect();
    let config = laplace_probe_sdk::ProbeSessionConfig {
        max_depth: 100_000,
        write_ard: true,
        output_dir: std::env::temp_dir().to_string_lossy().into_owned(),
    };

    laplace_probe_sdk::run_verification_from(&events, "r2d2_external_lock_get", &config);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Scenario 4: 명시적 AB-BA (수동 스레드 분기)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 수동 AB-BA 구성: Thread 0과 Thread 1이 명시적으로 다른 lock 순서를 사용.
/// 이것은 매크로가 아닌 수동 테스트로 구성한다.
///
/// Thread 0: external_lock → pool.get() (A→B)
/// Thread 1: pool.get() → external_lock (B→A)
///
/// Expected: BugFound (AB-BA deadlock)
use laplace_probe_sdk::{
    run_verification_from, set_probe_sender, set_probe_thread_id, ProbeEvent, ProbeSessionConfig,
};
use std::sync::mpsc;

#[test]
fn r2d2_manual_ab_ba_deadlock() {
    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(8192);

    let pool = Pool::builder()
        .max_size(8)
        .min_idle(Some(4))
        .test_on_check_out(false)
        .build(MockManager)
        .unwrap();
    let pool = Arc::new(pool);

    let external_lock = Arc::new(TrackedStdMutex::new(0u64, "user_state"));

    let mut handles = Vec::new();

    // Thread 0: external_lock(A) → pool.get()(B) — A→B 순서
    {
        let p = pool.clone();
        let ext = external_lock.clone();
        let tx2 = tx.clone();
        handles.push(std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(0u64);

            let _guard = ext.lock(); // Lock A
            let conn = p.get().unwrap(); // Lock B (r2d2 internal)
            let _ = *conn;
            drop(conn); // Release B (put_back)
            drop(_guard); // Release A
        }));
    }

    // Thread 1: pool.get()(B) → external_lock(A) — B→A 순서 (역순!)
    {
        let p = pool.clone();
        let ext = external_lock.clone();
        let tx2 = tx.clone();
        handles.push(std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(1u64);

            let conn = p.get().unwrap(); // Lock B (r2d2 internal)
            let _guard = ext.lock(); // Lock A — 역순!
            let _ = *conn;
            drop(_guard); // Release A
            drop(conn); // Release B (put_back)
        }));
    }

    drop(tx);
    for h in handles {
        h.join().expect("thread panicked");
    }

    let events: Vec<ProbeEvent> = rx.into_iter().collect();
    println!("\n[r2d2-hunt] Collected {} events:", events.len());

    let config = ProbeSessionConfig {
        max_depth: 100_000,
        write_ard: true,
        output_dir: std::env::temp_dir().to_string_lossy().into_owned(),
    };

    // AB-BA → BugFound 기대
    run_verification_from(&events, "r2d2_manual_ab_ba", &config).assert_bug();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Scenario 5: 풀 소진 경합 (max_size=2, 3 스레드)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

struct SmallR2d2State {
    pool: Pool<MockManager>,
}

impl Default for SmallR2d2State {
    fn default() -> Self {
        let pool = Pool::builder()
            .max_size(2)
            .min_idle(Some(2))
            .test_on_check_out(false)
            .build(MockManager)
            .unwrap();
        Self { pool }
    }
}

/// 3 스레드가 max_size=2 풀에서 동시 get().
/// 동기식이므로 1 스레드는 blocking wait (Condvar).
///
/// max_depth = 100_000
#[test]
fn r2d2_pool_exhaustion() {
    let (tx, rx) = std::sync::mpsc::sync_channel(8192);
    let state = Arc::new(SmallR2d2State::default());
    let mut handles = Vec::new();

    for thread_id in 0..3 {
        let s = state.clone();
        let tx2 = tx.clone();
        handles.push(std::thread::spawn(move || {
            laplace_probe_sdk::set_probe_sender(tx2);
            laplace_probe_sdk::set_probe_thread_id(thread_id as u64);

            let conn = s.pool.get().unwrap();
            let _ = *conn;
        }));
    }

    drop(tx);
    for h in handles {
        h.join().expect("thread panicked");
    }

    let events: Vec<_> = rx.into_iter().collect();
    let config = laplace_probe_sdk::ProbeSessionConfig {
        max_depth: 100_000,
        write_ard: true,
        output_dir: std::env::temp_dir().to_string_lossy().into_owned(),
    };

    laplace_probe_sdk::run_verification_from(&events, "r2d2_pool_exhaustion", &config);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Scenario 6: baseline (단일 스레드)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 단일 스레드 순차 접근. 경합 없음. CLEAN 기대.
#[test]
fn r2d2_single_thread() {
    let (tx, rx) = std::sync::mpsc::sync_channel(8192);
    let state = Arc::new(R2d2PoolState::default());
    let mut handles = Vec::new();

    for thread_id in 0..1 {
        let s = state.clone();
        let tx2 = tx.clone();
        handles.push(std::thread::spawn(move || {
            laplace_probe_sdk::set_probe_sender(tx2);
            laplace_probe_sdk::set_probe_thread_id(thread_id as u64);

            let conn = s.pool.get().unwrap();
            let _ = *conn;
        }));
    }

    drop(tx);
    for h in handles {
        h.join().expect("thread panicked");
    }

    let events: Vec<_> = rx.into_iter().collect();
    let config = laplace_probe_sdk::ProbeSessionConfig {
        max_depth: 100_000,
        write_ard: true,
        output_dir: std::env::temp_dir().to_string_lossy().into_owned(),
    };

    laplace_probe_sdk::run_verification_from(&events, "r2d2_single_thread", &config);
}
