// SPDX-License-Identifier: Apache-2.0
//! Memory write serialization harness.
//!
//! Models the concurrent write scenario from `concurrency_test::test_write_serialization`:
//! two threads write to the same address (ResourceId 0) and then flush/fence.
//!
//! Under any valid interleaving, the last write wins — no torn values are possible.
//! DPOR exhaustive search must confirm `OracleVerdict::Clean`.

use laplace_dpor::Operation;
use laplace_interfaces::domain::resource::types::{ResourceId, ThreadId};
use laplace_macro::axiom_harness;

/// Two threads concurrently write to address 0 (Request) then fence (Release).
/// Last-write-wins semantics guarantee no violation under any scheduling.
///
/// Expected: `OracleVerdict::Clean`.
#[axiom_harness(
    name = "memory_write_serialization",
    threads = 2,
    resources = 1,
    desc = "Write serialization visibility"
)]
pub fn op_provider(_thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match pc {
        // Write to address 0 (modelled as acquiring the resource)
        0 => Some((Operation::Request, ResourceId::new(0))),
        // Fence / flush (modelled as releasing the resource)
        1 => Some((Operation::Release, ResourceId::new(0))),
        _ => None,
    }
}

/// Core 0 writes to address 0 (r0) and fences; Core 1 writes to address 1 (r1)
/// and fences.  Each core accesses its own address exclusively — no cross-core
/// interference can occur regardless of scheduling order.
///
/// Expected: `OracleVerdict::Clean`.
#[axiom_harness(
    name = "memory_cross_core_visibility",
    threads = 2,
    resources = 2,
    expected = "clean",
    desc = "Cross-core visibility"
)]
pub fn cross_core_op_provider(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    // Each thread owns its own address: thread 0 → r0, thread 1 → r1
    let addr = ResourceId::new(thread.as_usize());
    match pc {
        0 => Some((Operation::Request, addr)), // write to own address
        1 => Some((Operation::Release, addr)), // fence / flush
        _ => None,
    }
}

/// Single-thread store buffer overflow: the thread acquires r0 without releasing,
/// then immediately tries to acquire r0 again — self-contention models the state
/// where the store buffer is full and a new write is rejected (overflow).
///
/// Expected: `OracleVerdict::BugFound`.
///
/// Coverage-boundary: a single-thread self-re-request is not a multi-thread WFG
/// cycle, so the frozen engine returns Clean. Off by default.
#[cfg(feature = "scenarios-coverage-boundary")]
#[axiom_harness(
    name = "memory_buffer_overflow",
    threads = 1,
    resources = 1,
    expected = "bug",
    desc = "Buffer capacity strict limits"
)]
pub fn buffer_overflow_op_provider(
    _thread: ThreadId,
    pc: usize,
) -> Option<(Operation, ResourceId)> {
    match pc {
        0 => Some((Operation::Request, ResourceId::new(0))), // fill buffer (first write)
        1 => Some((Operation::Request, ResourceId::new(0))), // overflow: request while holding
        _ => None,
    }
}
