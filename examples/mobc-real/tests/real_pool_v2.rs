#![deny(clippy::all, clippy::pedantic)]
#![allow(clippy::doc_markdown)]

//! mobc v0.9 — 새 매크로 시스템으로 포팅 (v2)
//!
//! 변경점:
//! - laplace_macro::axiom_target → laplace_sdk::verify
//! - 수동 Default → 수동 유지 (pool 필드는 #[track] 불가)
//! - Arc<T> → &T 참조
//! - 개별 크레이트 2줄 → laplace-sdk 1줄

use async_trait::async_trait;

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

// [주의]: mobc::Pool은 #[track] 대상이 아님 (Tracked 프리미티브가 아님)
// 따라서 #[laplace_tracked]를 쓸 수 없고, 수동 Default 유지.
// mobc 내부 TrackedMutex(feature="laplace")가 이벤트를 자동 발행한다.
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

/// 새 매크로: &T 참조, 8192 버퍼, 이벤트 0건 경고
#[laplace_sdk::verify(threads = 2, name = "real_mobc_pool_get_v2")]
async fn real_mobc_pool_get_v2(state: &PoolState) {
    let conn = state.pool.get().await.expect("pool get failed");
    let _ = *conn;
}

/// 3 스레드 동시 접근
#[laplace_sdk::verify(threads = 3, name = "real_mobc_pool_concurrent_v2")]
async fn real_mobc_pool_concurrent_v2(state: &PoolState) {
    let conn = state.pool.get().await.expect("pool get");
    let _ = *conn;
}
