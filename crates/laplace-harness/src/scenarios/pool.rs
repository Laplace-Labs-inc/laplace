// SPDX-License-Identifier: Apache-2.0
//! Thread-pool preemption fairness harness.
//!
//! Models a tiered pool: Free (T0), Standard (T1), Enterprise (T2).
//!
//! | Tier       | Thread | Resource access          |
//! |------------|--------|--------------------------|
//! | Free       | T0     | r0 only (Request/Release) |
//! | Standard   | T1     | r0 then r1 (Request/Release each) |
//! | Enterprise | T2     | r1 only (Request/Release) |
//!
//! The strict tier ordering means no circular dependency forms: T0 only uses r0,
//! T2 only uses r1, and T1 uses both in ascending order (r0 before r1).
//! Because no thread holds a higher resource while requesting a lower one,
//! no cyclic wait can occur — deadlock is impossible.
//!
//! Expected: `OracleVerdict::Clean`.

use laplace_core::domain::resource::{ResourceId, ThreadId};
use laplace_dpor::Operation;
use laplace_macro::axiom_harness;

/// Tiered preemption model — strict hierarchy prevents infinite oscillation.
///
/// Expected: `OracleVerdict::Clean`.
#[axiom_harness(
    name = "pool_preemption_fairness",
    threads = 3,
    resources = 2,
    expected = "clean",
    desc = "Tiered preemption"
)]
pub fn op_provider(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match thread.as_usize() {
        0 => match pc {
            // Free tier: uses r0 only
            0 => Some((Operation::Request, ResourceId::new(0))),
            1 => Some((Operation::Release, ResourceId::new(0))),
            _ => None,
        },
        1 => match pc {
            // Standard tier: acquires r0 then r1 in order (no inversion)
            0 => Some((Operation::Request, ResourceId::new(0))),
            1 => Some((Operation::Request, ResourceId::new(1))),
            2 => Some((Operation::Release, ResourceId::new(1))),
            3 => Some((Operation::Release, ResourceId::new(0))),
            _ => None,
        },
        2 => match pc {
            // Enterprise tier: uses r1 only
            0 => Some((Operation::Request, ResourceId::new(1))),
            1 => Some((Operation::Release, ResourceId::new(1))),
            _ => None,
        },
        _ => None,
    }
}
