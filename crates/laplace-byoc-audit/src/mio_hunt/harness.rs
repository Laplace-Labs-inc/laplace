// SPDX-License-Identifier: Apache-2.0
//! [DX 관찰 포인트]
//! - "버그 없음" 확인이 Ki-DPOR의 유효한 사용 사례인가?
//! - CLEAN 결과는 "시간 낭비"인가 "안전 증명"인가?

use laplace_core::domain::resource::{ResourceId, ThreadId};
use laplace_dpor::Operation;
use laplace_macro::axiom_harness;

// R0 = io Mutex, R1 = pool Mutex

/// 시나리오 1: 두 스레드 모두 io→pool 순서 (mio 실제 구현과 동일)
/// CLEAN 기대 — 일관된 순서이므로 교착 없음
#[cfg(feature = "laplace")]
#[axiom_harness(
    name = "mio_consistent_io_pool_ordering",
    threads = 2,
    resources = 2,
    desc = "io→pool 일관 순서 검증 — 실제 mio 코드와 동일, CLEAN 기대",
    expected = "clean"
)]
pub fn consistent_ordering_provider(
    _thread: ThreadId,
    pc: usize,
) -> Option<(Operation, ResourceId)> {
    match pc {
        0 => Some((Operation::Request, ResourceId::new(0))),
        1 => Some((Operation::Request, ResourceId::new(1))),
        2 => Some((Operation::Release, ResourceId::new(1))),
        3 => Some((Operation::Release, ResourceId::new(0))),
        _ => None,
    }
}

/// 시나리오 2: io/pool 역순 조합 (가상의 잘못된 구현)
/// BUG 기대 — AB-BA 교착
#[cfg(feature = "laplace")]
#[axiom_harness(
    name = "mio_hypothetical_reversed_ordering",
    threads = 2,
    resources = 2,
    desc = "역순 io/pool 가상 시나리오 — AB-BA 교착, 실제 mio는 이를 피함",
    expected = "bug"
)]
pub fn reversed_ordering_provider(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
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

/// 시나리오 3: connecting 플래그 + io 조합 TOCTOU
/// R0 = io Mutex, R1 = connecting AtomicBool
#[cfg(feature = "laplace")]
#[axiom_harness(
    name = "mio_connecting_io_toctou",
    threads = 2,
    resources = 2,
    desc = "connecting AtomicBool + io Mutex TOCTOU 탐색 — starvation 가능성",
    expected = "clean"
)]
pub fn connecting_toctou_provider(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match (thread.as_usize(), pc) {
        (0, 0) => Some((Operation::Request, ResourceId::new(1))),
        (0, 1) => Some((Operation::Request, ResourceId::new(0))),
        (0, 2) => Some((Operation::Release, ResourceId::new(0))),
        (0, 3) => Some((Operation::Release, ResourceId::new(1))),
        (1, 0) => Some((Operation::Request, ResourceId::new(1))),
        (1, 1) => Some((Operation::Request, ResourceId::new(0))),
        (1, 2) => Some((Operation::Release, ResourceId::new(0))),
        (1, 3) => Some((Operation::Release, ResourceId::new(1))),
        _ => None,
    }
}
