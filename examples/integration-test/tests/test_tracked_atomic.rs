//! Scenario 4: AtomicU64 — CLEAN
//!
//! 검증 항목:
//! - AtomicU64 → TrackedAtomicU64 교체
//! - load() → ProbeEvent::AtomicLoad 이벤트 발행
//! - store() → ProbeEvent::AtomicStore 이벤트 발행
//! - fetch_add() → ProbeEvent::AtomicRmw 이벤트 발행
//! - 단일 atomic 변수 → 교착 불가 → CLEAN

use laplace_sdk::prelude::*;
use std::sync::atomic::Ordering;

#[laplace_tracked]
struct AtomicState {
    #[track]
    counter: AtomicU64,
}

/// 2 스레드가 동시에 fetch_add.
/// 단일 자원 → 교착 불가 → CLEAN.
#[laplace_sdk::verify(threads = 2)]
async fn test_atomic_fetch_add_clean(state: &AtomicState) {
    state.counter.fetch_add(1, Ordering::SeqCst);
}

/// 2 스레드: load + store.
/// 단일 자원 → 교착 불가 → CLEAN.
/// (하지만 Read+Write는 충돌로 분류되어 DPOR가 인터리빙을 탐색한다)
#[laplace_sdk::verify(threads = 2)]
async fn test_atomic_load_store_clean(state: &AtomicState) {
    let _ = state.counter.load(Ordering::SeqCst);
    state.counter.store(100, Ordering::SeqCst);
}
