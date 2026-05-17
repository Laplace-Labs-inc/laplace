//! mio named_pipe mock-derived harnesses.
//!
//! R0 = io mutex, R1 = pool/connect state.

use laplace_core::domain::resource::{ResourceId, ThreadId};
use laplace_dpor::Operation;
use laplace_macro::axiom_harness;

#[axiom_harness(
    name = "mio_consistent_io_pool_ordering",
    threads = 2,
    resources = 2,
    desc = "io→pool 일관 순서 검증 - 실제 mio 코드와 동일, CLEAN 기대",
    expected = "clean"
)]
pub fn mio_consistent_io_pool_ordering(
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

#[axiom_harness(
    name = "mio_hypothetical_reversed_ordering",
    threads = 2,
    resources = 2,
    desc = "역순 io/pool 가상 시나리오 - AB-BA 교착, 실제 mio는 이를 피함",
    expected = "bug"
)]
pub fn mio_hypothetical_reversed_ordering(
    thread: ThreadId,
    pc: usize,
) -> Option<(Operation, ResourceId)> {
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

#[axiom_harness(
    name = "mio_connecting_io_toctou",
    threads = 2,
    resources = 2,
    desc = "connecting AtomicBool + io Mutex TOCTOU 탐색",
    expected = "clean"
)]
pub fn mio_connecting_io_toctou(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
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
