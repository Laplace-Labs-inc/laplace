// SPDX-License-Identifier: Apache-2.0
//! parking_lot external lock-ordering harnesses.
//!
//! 공개 이슈(#212, #489, #518)와 겹치지 않는 Condvar 교차 경로 + ABBA 경로를
//! 모델링해 Ki-DPOR로 검증한다.

use laplace_core::domain::resource::{ResourceId, ThreadId};
use laplace_dpor::Operation;
use laplace_macro::axiom_harness;

#[axiom_harness(
    name = "parking_lot_condvar_rwlock_cross",
    threads = 2,
    resources = 2,
    desc = "Condvar wait + RwLock write cross-path - 미공개 lock-ordering 경로",
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
