// SPDX-License-Identifier: Apache-2.0
//! Harness 방식 — op_provider로 추상 락 순서 모델링
//!
//! [DX 관찰 포인트]:
//!   - op_provider 작성이 직관적인가?
//!   - 실제 parking_lot API와 얼마나 일치하는가?
//!   - 어떤 부분이 부자연스러운가?

use laplace_core::domain::resource::{ResourceId, ThreadId};
use laplace_dpor::Operation;
use laplace_macro::axiom_harness;

// ── 시나리오 1: Condvar + RwLock 교차 (미공개 타겟) ──────────────────────────

/// R0 = parking_lot_rwlock (write lock)
/// R1 = parking_lot_condvar_mutex (내부 Mutex)
///
/// Thread 0 (writer): rwlock.write() → condvar.wait(mutex) 순서
/// Thread 1 (notifier): condvar_mutex.lock() → rwlock.read() 순서 (역!)
///
/// 이 순서 역전이 미공개 교착을 유발하는지 Ki-DPOR로 탐색.
/// [공개 버그 #489와 다른 점]: upgrade 경로가 아닌 Condvar 경유 경로
#[cfg(feature = "laplace")]
#[axiom_harness(
    name = "parking_lot_condvar_rwlock_cross",
    threads = 2,
    resources = 2,
    desc = "Condvar wait + RwLock write cross-path - 미공개 lock-ordering 경로",
    expected = "bug"
)]
pub fn condvar_rwlock_op_provider(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match (thread.as_usize(), pc) {
        // Thread 0: RwLock.write(R0) → Condvar.wait → 내부 Mutex(R1)
        (0, 0) => Some((Operation::Request, ResourceId::new(0))),
        (0, 1) => Some((Operation::Request, ResourceId::new(1))),
        (0, 2) => Some((Operation::Release, ResourceId::new(1))),
        (0, 3) => Some((Operation::Release, ResourceId::new(0))),

        // Thread 1 (역순!): Condvar.notify가 내부 Mutex(R1) 먼저 → RwLock.read(R0)
        (1, 0) => Some((Operation::Request, ResourceId::new(1))),
        (1, 1) => Some((Operation::Request, ResourceId::new(0))),
        (1, 2) => Some((Operation::Release, ResourceId::new(0))),
        (1, 3) => Some((Operation::Release, ResourceId::new(1))),

        _ => None,
    }
}

// ── 시나리오 2: 다중 RwLock AB-BA (Condvar 없이) ─────────────────────────────

#[cfg(feature = "laplace")]
#[axiom_harness(
    name = "parking_lot_rwlock_abba",
    threads = 2,
    resources = 2,
    desc = "두 RwLock AB-BA - #212/#489와 다른 read/write 조합 탐색",
    expected = "bug"
)]
pub fn rwlock_abba_op_provider(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match (thread.as_usize(), pc) {
        (0, 0) => Some((Operation::Request, ResourceId::new(0))),
        (0, 1) => Some((Operation::Request, ResourceId::new(1))),
        (0, 2) => Some((Operation::Release, ResourceId::new(1))),
        (0, 3) => Some((Operation::Release, ResourceId::new(0))),

        (1, 0) => Some((Operation::Request, ResourceId::new(1))),
        (1, 1) => Some((Operation::Request, ResourceId::new(0))),
        (1, 2) => Some((Operation::Release, ResourceId::new(0))),
        (1, 3) => Some((Operation::Release, ResourceId::new(1))),

        _ => None,
    }
}
