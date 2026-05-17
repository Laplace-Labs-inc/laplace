// SPDX-License-Identifier: Apache-2.0
//! Benchmark stability snapshot harness.
//!
//! Models a stability analyzer racing with a writer thread over a shared
//! metrics resource (r0).  The writer acquires r0 to update metrics; the
//! analyzer acquires r0 to take a consistent snapshot.  Mutual exclusion via
//! the lock prevents torn reads, so the snapshot is always coherent.
//!
//! Expected: `OracleVerdict::Clean`.

use laplace_core::domain::resource::{ResourceId, ThreadId};
use laplace_dpor::Operation;
use laplace_macro::axiom_harness;

/// Writer (T0) and analyzer (T1) race for the metrics lock — proves snapshots are clean.
///
/// Expected: `OracleVerdict::Clean`.
#[axiom_harness(
    name = "benchmark_stability_snapshot",
    threads = 2,
    resources = 1,
    expected = "clean",
    desc = "Stability analyzer snapshot race"
)]
pub fn op_provider(_thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match pc {
        0 => Some((Operation::Request, ResourceId::new(0))), // acquire metrics lock
        1 => Some((Operation::Release, ResourceId::new(0))), // release metrics lock
        _ => None,
    }
}
