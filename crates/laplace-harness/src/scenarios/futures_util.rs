//! futures-util async mutex harnesses.
//!
//! R0 = futures::lock::Mutex 외부 잠금 상태(IS_LOCKED 비트)
//! R1 = 내부 waiter slab(`StdMutex<Slab<Waiter>>`) 접근
//!
//! 공개 이슈(RUSTSEC-2020-0059/0062, issue #2133)를 제외하고,
//! cancel + waiter 순서로 인한 starvation 경로를 모델링한다.

use laplace_core::domain::resource::{ResourceId, ThreadId};
use laplace_dpor::Operation;
use laplace_macro::axiom_harness;

#[axiom_harness(
    name = "futures_mutex_starvation_3thread",
    threads = 3,
    resources = 2,
    desc = "stale waiter starvation - unlock이 취소된 waiter를 깨워 다음 waiter를 스킵",
    expected = "bug"
)]
pub fn futures_mutex_starvation(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match (thread.as_usize(), pc) {
        // T0 lock holder
        (0, 0) => Some((Operation::Request, ResourceId::new(0))),
        (0, 1) => Some((Operation::Request, ResourceId::new(1))),
        (0, 2) => Some((Operation::Release, ResourceId::new(1))),
        (0, 3) => Some((Operation::Release, ResourceId::new(0))),

        // T1 waiter + cancel(보수적 release)
        (1, 0) => Some((Operation::Request, ResourceId::new(1))),
        (1, 1) => Some((Operation::Release, ResourceId::new(1))),
        (1, 2) => Some((Operation::Request, ResourceId::new(0))),
        (1, 3) => Some((Operation::Release, ResourceId::new(0))),

        // T2 victim waiter
        (2, 0) => Some((Operation::Request, ResourceId::new(1))),
        (2, 1) => Some((Operation::Release, ResourceId::new(1))),
        (2, 2) => Some((Operation::Request, ResourceId::new(0))),

        _ => None,
    }
}

#[axiom_harness(
    name = "futures_mutex_basic_contention",
    threads = 2,
    resources = 2,
    desc = "기본 2-thread 기준선 - 각 스레드가 서로 다른 자원 요청/해제 (clean)",
    expected = "clean"
)]
pub fn futures_mutex_basic_contention(
    thread: ThreadId,
    pc: usize,
) -> Option<(Operation, ResourceId)> {
    match thread.as_usize() {
        0 => match pc {
            0 => Some((Operation::Request, ResourceId::new(0))),
            1 => Some((Operation::Release, ResourceId::new(0))),
            _ => None,
        },
        1 => match pc {
            0 => Some((Operation::Request, ResourceId::new(1))),
            1 => Some((Operation::Release, ResourceId::new(1))),
            _ => None,
        },
        _ => None,
    }
}
