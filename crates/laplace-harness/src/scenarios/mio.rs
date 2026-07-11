// SPDX-License-Identifier: Apache-2.0
//! mio named_pipe mock-derived harnesses.
//!
//! R0 = io mutex, R1 = pool/connect state.

use laplace_dpor::Operation;
use laplace_interfaces::domain::resource::types::{ResourceId, ThreadId};
use laplace_macro::axiom_harness;

#[axiom_harness(
    name = "mio_consistent_io_pool_ordering",
    threads = 2,
    resources = 2,
    desc = "io→pool consistent ordering check - matches actual mio code, expects CLEAN",
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
    desc = "reversed io/pool hypothetical scenario - AB-BA deadlock avoided by actual mio",
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
    desc = "connecting AtomicBool + io Mutex TOCTOU exploration",
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
