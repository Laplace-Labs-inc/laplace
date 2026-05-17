// SPDX-License-Identifier: Apache-2.0
//! CoreResourcePool harness — 3 threads competing for 2 resources.
//!
//! # Scenario
//!
//! Three virtual threads operate over two resources (r0, r1):
//!
//! | Thread | Resource | Sequence              |
//! |--------|----------|-----------------------|
//! | 0      | r0       | Request r0 → Release r0 → done |
//! | 1      | r1       | Request r1 → Release r1 → done |
//! | 2      | r0       | Request r0 → Release r0 → done |
//!
//! Thread 0 and Thread 2 both contend for r0.  Thread 1 holds r1 exclusively.
//!
//! # Why this is deadlock-free
//!
//! No circular wait can form: every thread holds at most one resource at a time,
//! and always releases it before terminating.  When t0 holds r0 and t2 is
//! blocked waiting for r0, t0 will release r0 allowing t2 to proceed —
//! the DPOR exhaustive search will confirm `OracleVerdict::Clean`.

use laplace_core::domain::resource::{ResourceId, ThreadId};
use laplace_dpor::Operation;
use laplace_macro::axiom_harness;

#[axiom_harness(
    name = "core_resource_pool",
    threads = 3,
    resources = 2,
    desc = "3 threads compete for 2 resources with acquire/release — proves no deadlock or double-assignment"
)]
pub fn op_provider(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    // Thread 0 and 2 → resource 0; Thread 1 → resource 1
    let resource = thread.as_usize() % 2;
    match pc {
        0 => Some((Operation::Request, ResourceId::new(resource))),
        1 => Some((Operation::Release, ResourceId::new(resource))),
        _ => None,
    }
}
