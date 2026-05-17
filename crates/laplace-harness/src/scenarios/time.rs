// SPDX-License-Identifier: Apache-2.0
//! Lamport clock atomic increment harness.
//!
//! Two threads contend for the shared logical clock (r0).  Each thread acquires
//! the clock, increments it atomically, then releases — ensuring a total order
//! on all increments with no lost updates.
//!
//! Expected: `OracleVerdict::Clean`.

use laplace_core::domain::resource::{ResourceId, ThreadId};
use laplace_dpor::Operation;
use laplace_macro::axiom_harness;

/// 2 threads atomically increment the Lamport clock — proves no increment is lost.
///
/// Expected: `OracleVerdict::Clean`.
#[axiom_harness(
    name = "time_lamport_ordering",
    threads = 2,
    resources = 1,
    expected = "clean",
    desc = "Lamport clock atomic increment"
)]
pub fn op_provider(_thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match pc {
        0 => Some((Operation::Request, ResourceId::new(0))), // acquire clock, increment
        1 => Some((Operation::Release, ResourceId::new(0))), // release clock
        _ => None,
    }
}
