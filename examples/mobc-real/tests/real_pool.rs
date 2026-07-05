#![deny(clippy::all, clippy::pedantic)]
#![allow(clippy::doc_markdown)]

//! mobc v0.9 (patched with TrackedMutex) 실제 Pool API를 사용한 Ki-DPOR 검증.
//!
//! mobc의 소스는 vendor/mobc/src/lib.rs에 있으며
//! TrackedMutex 패치가 적용된 상태다 (feature = "laplace").
//!
//! 분석 결과:
//! - 내부 Mutex 수: 1개 (SharedPool::internals)
//! - Mutex 이름: "internals"
//! - 복수 락 획득 경로: 없음 (단일 락만 사용)
//! - 예상 결과: CLEAN (AB-BA 불가능)

use async_trait::async_trait;
use laplace_macro::axiom_target;
use std::sync::Arc;

// ── MockManager — 테스트용 최소 연결 관리자 ────────────────────────────────────

#[derive(Debug, Clone)]
struct MockManager;

#[async_trait]
impl mobc::Manager for MockManager {
    type Connection = i64;
    type Error = std::io::Error;

    async fn connect(&self) -> Result<Self::Connection, Self::Error> {
        Ok(42) // 즉시 반환되는 더미 연결
    }

    async fn check(&self, conn: Self::Connection) -> Result<Self::Connection, Self::Error> {
        Ok(conn)
    }
}

// ── Scenario A: Pool::get() 동시 호출 (2-thread) ────────────────────────────────

struct PoolState {
    pool: mobc::Pool<MockManager>,
}

impl Default for PoolState {
    fn default() -> Self {
        let manager = MockManager;
        let pool = mobc::Pool::builder().max_open(4).build(manager);
        Self { pool }
    }
}

/// 실제 mobc Pool::get() 동시 호출.
/// mobc 내부 TrackedMutex (internals)가 이벤트를 자동으로 수집한다.
///
/// 단일 Mutex만 있으므로 AB-BA 불가능 → CLEAN 기대
#[axiom_target(threads = 2, name = "real_mobc_pool_get")]
async fn real_mobc_pool_get(state: Arc<PoolState>) {
    let conn = state.pool.get().await.expect("pool get failed");
    // connection 사용 시뮬레이션
    let _ = *conn;
    // conn drop → put_back() 자동 호출
}

// ── Scenario B: 3-thread Pool 동시 접근 ─────────────────────────────────────────

/// 3개 스레드가 동시에 get(). 단일 Mutex.
/// 어떤 인터리빙도 교착 불가 → CLEAN.
#[axiom_target(threads = 3, name = "real_mobc_pool_concurrent")]
async fn real_mobc_pool_concurrent(state: Arc<PoolState>) {
    let conn = state.pool.get().await.expect("pool get");
    let _ = *conn;
}
