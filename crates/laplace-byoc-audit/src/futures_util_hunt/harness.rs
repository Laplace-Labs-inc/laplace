//! futures-util Ki-DPOR 하네스 — stale waiter starvation 모델
//!
//! [DX 관찰 포인트]:
//! - async Mutex를 (Operation, ResourceId)로 표현하는 것이 얼마나 부자연스러운가?
//! - parking_lot 하네스와 비교했을 때 모델 신뢰도 차이는?
//! - 취소(cancel) 시나리오를 Ki-DPOR로 표현하는 것이 가능한가?

use laplace_core::domain::resource::{ResourceId, ThreadId};
use laplace_dpor::Operation;
use laplace_macro::axiom_harness;

#[cfg(feature = "laplace")]
#[axiom_harness(
    name = "futures_mutex_starvation_3thread",
    threads = 3,
    resources = 2,
    desc = "stale waiter starvation - unlock이 취소된 waiter를 깨워 다음 waiter를 스킵",
    expected = "bug"
)]
pub fn starvation_op_provider(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match (thread.as_usize(), pc) {
        (0, 0) => Some((Operation::Request, ResourceId::new(0))),
        (0, 1) => Some((Operation::Request, ResourceId::new(1))),
        (0, 2) => Some((Operation::Release, ResourceId::new(1))),
        (0, 3) => Some((Operation::Release, ResourceId::new(0))),

        (1, 0) => Some((Operation::Request, ResourceId::new(1))),
        (1, 1) => Some((Operation::Release, ResourceId::new(1))),
        (1, 2) => Some((Operation::Request, ResourceId::new(0))),
        (1, 3) => Some((Operation::Release, ResourceId::new(0))),

        (2, 0) => Some((Operation::Request, ResourceId::new(1))),
        (2, 1) => Some((Operation::Release, ResourceId::new(1))),
        (2, 2) => Some((Operation::Request, ResourceId::new(0))),

        _ => None,
    }
}

#[cfg(feature = "laplace")]
#[axiom_harness(
    name = "futures_mutex_basic_contention",
    threads = 2,
    resources = 2,
    desc = "기본 2-thread 기준선 - 각 스레드가 서로 다른 자원 요청/해제 (clean)",
    expected = "clean"
)]
pub fn basic_contention_op_provider(
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
