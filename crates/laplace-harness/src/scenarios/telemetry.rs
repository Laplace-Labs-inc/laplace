//! Telemetry ring buffer concurrency harness.
//!
//! Models the concurrent push scenario from `telemetry_twin_test` (S-TEL3 / S-TEL4):
//! two threads compete for the ring buffer's write-lock (ResourceId 0), push an
//! event, then release the lock.
//!
//! Because exactly one thread holds the write-lock at a time, no torn snapshot
//! or data race can occur.  DPOR exhaustive search must confirm `OracleVerdict::Clean`.

use laplace_core::domain::resource::{ResourceId, ThreadId};
use laplace_dpor::Operation;
use laplace_macro::axiom_harness;

/// Two threads race to acquire the ring buffer write-lock (ResourceId 0),
/// push one event (while holding the lock), then release.
///
/// Expected: `OracleVerdict::Clean`.
#[axiom_harness(
    name = "telemetry_ring_buffer_concurrent",
    threads = 2,
    resources = 1,
    desc = "Concurrent ring buffer push"
)]
pub fn op_provider(_thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match pc {
        // Acquire write-lock to push an event into the ring buffer
        0 => Some((Operation::Request, ResourceId::new(0))),
        // Release write-lock after push completes
        1 => Some((Operation::Release, ResourceId::new(0))),
        _ => None,
    }
}

/// Two threads concurrently call `fetch_add` on the telemetry counter (r0).
/// Mutual exclusion on r0 ensures no increment is lost — final count equals
/// the sum of both threads' increments.
///
/// Expected: `OracleVerdict::Clean`.
#[axiom_harness(
    name = "telemetry_atomic_increment",
    threads = 2,
    resources = 1,
    expected = "clean",
    desc = "Atomic counter concurrent increment"
)]
pub fn atomic_increment_op_provider(
    _thread: ThreadId,
    pc: usize,
) -> Option<(Operation, ResourceId)> {
    match pc {
        0 => Some((Operation::Request, ResourceId::new(0))), // fetch_add — acquire counter
        1 => Some((Operation::Release, ResourceId::new(0))), // release counter
        _ => None,
    }
}
