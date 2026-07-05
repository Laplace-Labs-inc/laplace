#![deny(clippy::all, clippy::pedantic)]
#![allow(clippy::doc_markdown)]

//! bb8 실전 버그 사냥 — Ki-DPOR 최대 깊이 탐색
//!
//! bb8 v0.8.1의 내부 `Mutex<PoolInternals>`가 `TrackedStdMutex`로 패치됨.
//! 모든 pool.get(), put_back(), reap() 호출이 자동으로 Lock/Unlock 이벤트를 방출.
//! Ki-DPOR가 이 이벤트들의 인터리빙을 전수 탐색하여 동시성 버그를 찾는다.
//!
//! 탐색 깊이: max_depth = 100_000 (최대)

use async_trait::async_trait;

// ── MockManager — 테스트용 즉시 반환 커넥션 관리자 ─────────────────────────────

#[derive(Debug)]
struct MockManager;

#[async_trait]
impl bb8::ManageConnection for MockManager {
    type Connection = i64;
    type Error = std::io::Error;

    async fn connect(&self) -> Result<Self::Connection, Self::Error> {
        Ok(42)
    }

    async fn is_valid(&self, _conn: &mut Self::Connection) -> Result<(), Self::Error> {
        Ok(())
    }

    fn has_broken(&self, _conn: &mut Self::Connection) -> bool {
        false
    }
}

// ── SharedState ───────────────────────────────────────────────────────────────

struct Bb8PoolState {
    pool: bb8::Pool<MockManager>,
}

// [중요] Default 수동 구현 — bb8::Pool은 async build이므로 sync Default 불가.
// 대신 tokio::runtime::Runtime을 사용하여 동기적으로 빌드.
impl Default for Bb8PoolState {
    fn default() -> Self {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");
        let pool = rt.block_on(async {
            bb8::Pool::builder()
                .max_size(4)
                .min_idle(Some(0))
                .test_on_check_out(false)
                .build(MockManager)
                .await
                .expect("Failed to build pool")
        });
        Self { pool }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Scenario 1: 기본 pool.get() 동시 호출 (2 스레드)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 2 스레드가 동시에 pool.get()을 호출.
/// bb8 내부 `Mutex<PoolInternals>` lock이 TrackedStdMutex로 추적됨.
/// Ki-DPOR가 get→lock→pop/put 인터리빙을 전수 탐색.
#[laplace_sdk::verify(threads = 2, name = "bb8_pool_get_2thread")]
async fn bb8_pool_get_2thread(state: &Bb8PoolState) {
    let conn = state.pool.get().await.expect("pool.get() failed");
    let _ = *conn; // 커넥션 사용
                   // conn Drop → put_back() → lock → put() → notify_one()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Scenario 2: 3 스레드 동시 checkout (max_size=4, 경합 높음)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 3 스레드가 동시에 pool.get()을 호출.
/// max_size=4이므로 3개 모두 성공해야 하나, 내부 lock 경합이 발생.
/// pending_conns 산술 (증감)에서 경합 시 불일치 가능.
#[laplace_sdk::verify(threads = 3, name = "bb8_pool_get_3thread")]
async fn bb8_pool_get_3thread(state: &Bb8PoolState) {
    let conn = state.pool.get().await.expect("pool.get() failed");
    let _ = *conn;
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Scenario 3: get + state 혼합 (읽기/쓰기 경합)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// pool.get() + pool.state() 동시 호출.
/// state()도 internals.lock()을 잡으므로 get()과 경합.
#[laplace_sdk::verify(threads = 2, name = "bb8_get_vs_state")]
async fn bb8_get_vs_state(state: &Bb8PoolState) {
    let conn = state.pool.get().await.expect("pool.get() failed");
    let _ = *conn;
    let _s = state.pool.state();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Scenario 4: 풀 소진 경합 (max_size=2, 3 스레드)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

struct SmallPoolState {
    pool: bb8::Pool<MockManager>,
}

impl Default for SmallPoolState {
    fn default() -> Self {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");
        let pool = rt.block_on(async {
            bb8::Pool::builder()
                .max_size(2)
                .min_idle(Some(0))
                .test_on_check_out(false)
                .build(MockManager)
                .await
                .expect("Failed to build pool")
        });
        Self { pool }
    }
}

/// 3 스레드가 max_size=2 풀에서 동시 get().
/// 1 스레드는 반드시 notify.notified().await 대기 상태에 진입.
/// put_back() → notify_one() 시점과 대기자 깨우기 사이의 경합을 탐색.
#[laplace_sdk::verify(threads = 3, name = "bb8_pool_exhaustion")]
async fn bb8_pool_exhaustion(state: &SmallPoolState) {
    let conn = state.pool.get().await.expect("pool.get() failed");
    let _ = *conn;
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Scenario 5: 단순 sequential get/drop (리그레션 베이스라인)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// 단일 스레드에서 sequential get/drop.
/// 리그레션 베이스라인 — 가장 간단한 경우.
#[laplace_sdk::verify(threads = 1, name = "bb8_single_thread_sequential")]
async fn bb8_single_thread_sequential(state: &Bb8PoolState) {
    let conn = state.pool.get().await.expect("pool.get() failed");
    let _ = *conn;
}
