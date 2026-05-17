//! Scenario 1: #[laplace_tracked] + #[laplace_sdk::verify] — 단일 Mutex CLEAN
//!
//! 검증 항목:
//! - #[laplace_tracked]가 Mutex<T> → TrackedMutex<T>로 교체하는가
//! - #[track]이 리소스 이름을 자동 생성하는가
//! - Default impl이 자동 생성되는가
//! - #[laplace_sdk::verify]가 &T 시그니처를 올바르게 처리하는가
//! - 단일 Mutex에서 CLEAN 결과를 반환하는가

use laplace_sdk::prelude::*;

#[laplace_tracked]
struct SingleCounter {
    #[track]
    counter: Mutex<i64>,
}

/// 2 스레드가 동시에 counter를 증가.
/// 단일 Mutex → AB-BA 불가능 → CLEAN.
#[laplace_sdk::verify(threads = 2)]
async fn test_single_mutex_clean(state: &SingleCounter) {
    let mut g = state.counter.lock().await;
    *g += 1;
}
