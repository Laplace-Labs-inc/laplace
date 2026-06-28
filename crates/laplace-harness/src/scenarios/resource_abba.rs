// SPDX-License-Identifier: Apache-2.0
//! AB-BA deadlock harness — classic circular wait between two threads.
//!
//! # Deadlock Pattern
//!
//! - Thread 0 (AB): Request R0 → Request R1 → Release R1 → Release R0
//! - Thread 1 (BA): Request R1 → Request R0 → Release R0 → Release R1
//!
//! If the threads interleave as T0 acquires R0 and T1 acquires R1, both then
//! block waiting for the resource held by the other — circular wait → deadlock.
//!
//! # Expected Result
//!
//! `OracleVerdict::BugFound` — DPOR will discover the deadlocking interleaving.

use laplace_dpor::Operation;
use laplace_interfaces::domain::resource::types::{ResourceId, ThreadId};
use laplace_macro::axiom_harness;

#[axiom_harness(
    name = "resource_abba_deadlock",
    threads = 2,
    resources = 2,
    desc = "Classic AB-BA deadlock scenario",
    expected = "bug"
)]
pub fn op_provider(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match thread.as_usize() {
        0 => match pc {
            // Thread 0: Request R0 → Request R1 → Release R1 → Release R0
            0 => Some((Operation::Request, ResourceId::new(0))),
            1 => Some((Operation::Request, ResourceId::new(1))),
            2 => Some((Operation::Release, ResourceId::new(1))),
            3 => Some((Operation::Release, ResourceId::new(0))),
            _ => None,
        },
        1 => match pc {
            // Thread 1: Request R1 → Request R0 → Release R0 → Release R1
            0 => Some((Operation::Request, ResourceId::new(1))),
            1 => Some((Operation::Request, ResourceId::new(0))),
            2 => Some((Operation::Release, ResourceId::new(0))),
            3 => Some((Operation::Release, ResourceId::new(1))),
            _ => None,
        },
        _ => None,
    }
}
