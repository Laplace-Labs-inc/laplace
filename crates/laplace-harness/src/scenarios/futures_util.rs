// SPDX-License-Identifier: Apache-2.0
//! futures-util async mutex harnesses.
//!
//! R0 = futures::lock::Mutex external lock state (IS_LOCKED bit)
//! R1 = access to the internal waiter slab (`StdMutex<Slab<Waiter>>`)
//!
//! Excluding the public issues (RUSTSEC-2020-0059/0062, issue #2133), this
//! models the starvation path caused by cancel and waiter ordering.

use laplace_dpor::Operation;
use laplace_interfaces::domain::resource::types::{ResourceId, ThreadId};
use laplace_macro::axiom_harness;

// Coverage-boundary: stale-waiter starvation (a fairness property), not a cyclic
// deadlock — the frozen engine returns Clean. Off by default.
#[cfg(feature = "scenarios-coverage-boundary")]
#[axiom_harness(
    name = "futures_mutex_starvation_3thread",
    threads = 3,
    resources = 2,
    desc = "stale waiter starvation - unlock wakes a cancelled waiter and skips the next waiter",
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
    desc = "basic 2-thread baseline - each thread requests/releases a different resource (clean)",
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
