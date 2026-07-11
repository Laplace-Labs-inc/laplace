// SPDX-License-Identifier: Apache-2.0
//! parking_lot external lock-ordering harnesses.
//!
//! Models Condvar cross-path and ABBA paths that do not overlap with the public
//! issues (#212, #489, #518), then verifies them with DPOR.

use laplace_dpor::Operation;
use laplace_interfaces::domain::resource::types::{ResourceId, ThreadId};
use laplace_macro::axiom_harness;

#[axiom_harness(
    name = "parking_lot_condvar_rwlock_cross",
    threads = 2,
    resources = 2,
    desc = "Condvar wait + RwLock write cross-path - unpublished lock-ordering path",
    expected = "bug"
)]
pub fn condvar_rwlock_op_provider(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
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
    name = "parking_lot_rwlock_abba",
    threads = 2,
    resources = 2,
    desc = "two RwLock AB-BA - explore a read/write combination different from #212/#489",
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
