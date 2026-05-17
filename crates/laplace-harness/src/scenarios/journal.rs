//! Journal concurrent log ordering harness.
//!
//! Three threads each write to their own dedicated log slot (r0, r1, r2).
//! No two threads share a slot, so no ordering conflict or data race can occur
//! regardless of scheduling.
//!
//! Expected: `OracleVerdict::Clean`.

use laplace_core::domain::resource::{ResourceId, ThreadId};
use laplace_dpor::Operation;
use laplace_macro::axiom_harness;

/// 3 threads write to independent log slots — proves concurrent log writes are safe.
///
/// Expected: `OracleVerdict::Clean`.
#[axiom_harness(
    name = "journal_concurrent_log_ordering",
    threads = 3,
    resources = 3,
    expected = "clean",
    desc = "Concurrent log ordering"
)]
pub fn op_provider(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    // Each thread owns its own slot: thread N → resource N
    let slot = ResourceId::new(thread.as_usize());
    match pc {
        0 => Some((Operation::Request, slot)), // acquire log slot, write entry
        1 => Some((Operation::Release, slot)), // release log slot
        _ => None,
    }
}
