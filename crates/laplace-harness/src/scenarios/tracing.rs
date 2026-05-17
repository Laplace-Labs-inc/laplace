//! Tracing causality acyclicity harness.
//!
//! Models two threads creating a causal cycle:
//!
//! - Thread 0 (A→B): Request r0 → Request r0 (tries to acquire while holding it)
//! - Thread 1 (B→A): Request r0 (blocked by T0-held r0)
//!
//! When T0 holds r0 and then tries to acquire r0 again (self-contention), and T1
//! is also waiting for r0, all threads end up blocked — the classic causal cycle
//! that DPOR will detect as a deadlock.
//!
//! Expected: `OracleVerdict::BugFound`.

use laplace_core::domain::resource::{ResourceId, ThreadId};
use laplace_dpor::Operation;
use laplace_macro::axiom_harness;

/// T0 creates A→B causality (Request r0 twice — self-deadlock).
/// T1 creates B→A causality (Request r0 while T0 holds it).
/// The intersection forms a cycle → deadlock detected.
///
/// Expected: `OracleVerdict::BugFound`.
#[axiom_harness(
    name = "tracing_causality_acyclicity",
    threads = 2,
    resources = 1,
    expected = "bug",
    desc = "Causality cycle detection"
)]
pub fn op_provider(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match thread.as_usize() {
        0 => match pc {
            // Thread 0: acquire r0 (A→B edge), then try to acquire r0 again
            // while holding it → self-deadlock, completing the cycle
            0 => Some((Operation::Request, ResourceId::new(0))),
            1 => Some((Operation::Request, ResourceId::new(0))),
            _ => None,
        },
        1 => match pc {
            // Thread 1: wait for r0 held by T0 (B→A edge)
            0 => Some((Operation::Request, ResourceId::new(0))),
            1 => Some((Operation::Release, ResourceId::new(0))),
            _ => None,
        },
        _ => None,
    }
}
