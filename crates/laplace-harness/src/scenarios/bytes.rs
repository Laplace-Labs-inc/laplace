// SPDX-License-Identifier: Apache-2.0
//! bytes lock-free refcount TOCTOU harnesses.
//!
//! R0 = shared allocation reference, R1 = is_unique/mutation intent.

use laplace_dpor::Operation;
use laplace_interfaces::domain::resource::types::{ResourceId, ThreadId};
use laplace_macro::axiom_harness;

#[axiom_harness(
    name = "bytes_is_unique_toctou",
    threads = 2,
    resources = 2,
    desc = "is_unique TOCTOU: clone이 to_mut 사이에 끼어드는 인터리빙 탐색",
    expected = "clean"
)]
pub fn bytes_is_unique_toctou(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match (thread.as_usize(), pc) {
        (0, 0) => Some((Operation::Request, ResourceId::new(0))),
        (0, 1) => Some((Operation::Release, ResourceId::new(0))),

        (1, 0) => Some((Operation::Read, ResourceId::new(1))),
        (1, 1) => Some((Operation::Write, ResourceId::new(1))),
        (1, 2) => Some((Operation::Release, ResourceId::new(0))),

        _ => None,
    }
}

#[axiom_harness(
    name = "bytes_three_thread_refcount",
    threads = 3,
    resources = 2,
    desc = "3-thread clone/to_mut/drop 인터리빙 탐색",
    expected = "clean"
)]
pub fn bytes_three_thread_refcount(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match (thread.as_usize(), pc) {
        (0, 0) => Some((Operation::Request, ResourceId::new(0))),
        (0, 1) => Some((Operation::Release, ResourceId::new(0))),

        (1, 0) => Some((Operation::Read, ResourceId::new(1))),
        (1, 1) => Some((Operation::Write, ResourceId::new(1))),
        (1, 2) => Some((Operation::Release, ResourceId::new(0))),

        (2, 0) => Some((Operation::Release, ResourceId::new(0))),

        _ => None,
    }
}
