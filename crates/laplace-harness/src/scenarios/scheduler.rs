// SPDX-License-Identifier: Apache-2.0
//! Scheduler liveness harness — round-robin fairness proof.
//!
//! Three threads share one scheduler slot (r0).  Under round-robin scheduling
//! every thread gets a turn; no thread can be permanently denied the slot.
//! DPOR exhaustive search must confirm `OracleVerdict::Clean`.

use laplace_core::domain::resource::{ResourceId, ThreadId};
use laplace_dpor::Operation;
use laplace_macro::axiom_harness;

/// 3 threads each acquire the scheduler slot (r0) for one quantum then release.
/// Round-robin ensures every thread completes without starvation.
///
/// Expected: `OracleVerdict::Clean`.
#[axiom_harness(
    name = "scheduler_liveness_roundrobin",
    threads = 3,
    resources = 1,
    expected = "clean",
    desc = "Round-robin fairness"
)]
pub fn op_provider(_thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match pc {
        0 => Some((Operation::Request, ResourceId::new(0))), // acquire slot
        1 => Some((Operation::Release, ResourceId::new(0))), // yield slot
        _ => None,
    }
}
