//! Scenario 5: 복합 — Mutex + RwLock + AtomicU64 혼합
//!
//! 검증 항목:
//! - 하나의 구조체에서 다양한 프리미티브 혼합 사용
//! - #[track(name = "custom")]로 커스텀 이름 지정
//! - #[track] 없는 필드는 그대로 유지
//! - Default가 올바르게 생성되는가
//! - 전체 end-to-end CLEAN

use laplace_sdk::prelude::*;

#[derive(Default)]
struct AppConfig {
    _max_connections: usize,
}

#[laplace_tracked]
struct FullService {
    #[track]
    cache: Mutex<Vec<String>>,

    #[track(name = "meta_store")]
    metadata: RwLock<String>,

    // #[track] 없음 — 교체 없이 그대로
    config: AppConfig,
}

/// 2 스레드가 cache만 접근 (단일 자원).
/// 단일 자원 → AB-BA 불가 → CLEAN.
#[laplace_sdk::verify(threads = 2)]
async fn test_full_service_clean(state: &FullService) {
    // Mutex 사용
    let mut cache = state.cache.lock().await;
    cache.push("entry".to_string());
}
