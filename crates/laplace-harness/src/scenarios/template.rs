// SPDX-License-Identifier: Apache-2.0
//! Template harness — basic single-resource request/release round-trip.
//!
//! Each of 2 threads requests resource 0 then releases it, then terminates.
//! DPOR exhaustive search must find no violations (OracleVerdict::Clean).

use laplace_core::domain::resource::{ResourceId, ThreadId};
use laplace_dpor::Operation;
use laplace_macro::axiom_harness;

#[axiom_harness(
    name = "template_harness",
    threads = 2,
    resources = 1,
    desc = "Each thread requests then releases resource 0 — verifies normal acquire/release cycle"
)]
pub fn op_provider(_thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match pc {
        0 => Some((Operation::Request, ResourceId::new(0))),
        1 => Some((Operation::Release, ResourceId::new(0))),
        _ => None,
    }
}
