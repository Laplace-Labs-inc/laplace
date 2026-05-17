//! bytes lock-free refcount TOCTOU 하네스
//!
//! [DX 관찰 포인트]
//! - lock-free 코드를 Request/Release로 표현하는 것이 자연스러운가?
//! - is_unique TOCTOU가 Ki-DPOR에서 포착되는가?

use laplace_core::domain::resource::{ResourceId, ThreadId};
use laplace_dpor::Operation;
use laplace_macro::axiom_harness;

#[cfg(feature = "laplace")]
#[axiom_harness(
    name = "bytes_is_unique_toctou",
    threads = 2,
    resources = 2,
    desc = "is_unique TOCTOU: clone이 to_mut 사이에 끼어드는 인터리빙 탐색",
    expected = "clean"
)]
pub fn is_unique_toctou_provider(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match (thread.as_usize(), pc) {
        // T0 (clone path): refcount++ then refcount--
        (0, 0) => Some((Operation::Request, ResourceId::new(0))),
        (0, 1) => Some((Operation::Release, ResourceId::new(0))),

        // T1 (to_mut path): is_unique read -> mutation write -> release
        (1, 0) => Some((Operation::Read, ResourceId::new(1))),
        (1, 1) => Some((Operation::Write, ResourceId::new(1))),
        (1, 2) => Some((Operation::Release, ResourceId::new(0))),

        _ => None,
    }
}

#[cfg(feature = "laplace")]
#[axiom_harness(
    name = "bytes_three_thread_refcount",
    threads = 3,
    resources = 2,
    desc = "3-thread clone/to_mut/drop 인터리빙 탐색",
    expected = "clean"
)]
pub fn three_thread_provider(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match (thread.as_usize(), pc) {
        // T0: clone
        (0, 0) => Some((Operation::Request, ResourceId::new(0))),
        (0, 1) => Some((Operation::Release, ResourceId::new(0))),

        // T1: to_mut
        (1, 0) => Some((Operation::Read, ResourceId::new(1))),
        (1, 1) => Some((Operation::Write, ResourceId::new(1))),
        (1, 2) => Some((Operation::Release, ResourceId::new(0))),

        // T2: drop
        (2, 0) => Some((Operation::Release, ResourceId::new(0))),

        _ => None,
    }
}
