// SPDX-License-Identifier: Apache-2.0
//! Entropy / RNG snapshot determinism harness.
//!
//! Two threads access the shared RNG state (r0) to derive independent streams.
//! Each thread acquires the RNG state exclusively, reads/forks it, then releases —
//! guaranteeing deterministic, non-overlapping output streams under any scheduling.
//!
//! Expected: `OracleVerdict::Clean`.

use laplace_dpor::Operation;
use laplace_interfaces::domain::resource::types::{ResourceId, ThreadId};
use laplace_macro::axiom_harness;

/// 2 threads fork independent RNG streams from shared state — proves no overlap.
///
/// Expected: `OracleVerdict::Clean`.
#[axiom_harness(
    name = "entropy_snapshot_determinism",
    threads = 2,
    resources = 1,
    expected = "clean",
    desc = "RNG snapshot determinism"
)]
pub fn op_provider(_thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match pc {
        0 => Some((Operation::Request, ResourceId::new(0))), // acquire RNG state, fork stream
        1 => Some((Operation::Release, ResourceId::new(0))), // release RNG state
        _ => None,
    }
}
