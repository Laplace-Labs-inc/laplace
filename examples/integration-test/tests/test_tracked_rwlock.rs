//! Scenario 3: RwLock — 읽기 동시성 CLEAN
//!
//! 검증 항목:
//! - RwLock<T> → TrackedRwLock<T> 교체
//! - read() → ProbeEvent::RwLockReadAcquired 이벤트 발행
//! - write() → ProbeEvent::RwLockWriteAcquired 이벤트 발행
//! - 읽기끼리는 DPOR 비충돌 (SharedRequest+SharedRequest = 비충돌)
//! - CLEAN 결과

use laplace_sdk::prelude::*;

#[laplace_tracked]
struct RwLockState {
    #[track]
    data: RwLock<Vec<u8>>,
}

/// 2 스레드가 동시에 read()만 호출.
/// RwLock 읽기는 공유 가능 → CLEAN.
#[laplace_sdk::verify(threads = 2)]
async fn test_rwlock_read_clean(state: &RwLockState) {
    let guard = state.data.read().await;
    let _len = guard.len();
}

/// 2 스레드가 동시에 write() 호출.
/// 단일 자원에 대한 배타 락 → AB-BA 불가 → CLEAN.
#[laplace_sdk::verify(threads = 2)]
async fn test_rwlock_write_clean(state: &RwLockState) {
    let mut guard = state.data.write().await;
    guard.push(42);
}
