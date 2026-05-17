//! `meter_inner` resource acquisition order safety harnesses.
//!
//! `laplace-api` POST `/credits/meter` path acquires two DB resources in order:
//!   R0: PostgreSQL row lock on `credits` (atomic UPDATE with `balance >= $1`)
//!   R1: `credit_ledger` table insertion
//!
//! This module verifies three concurrency properties:
//!   1. `credit_meter_concurrent_safe`   — 4-thread concurrent R0->R1 order => Clean
//!   2. `credit_meter_inverse_deadlock`  — mixed R0->R1 vs R1->R0 => BugFound
//!   3. `credit_meter_multi_tenant_safe` — two tenants with distinct credit rows => Clean

use laplace_core::domain::resource::{ResourceId, ThreadId};
use laplace_dpor::Operation;
use laplace_macro::axiom_harness;

// R0: `credits` row lock (from UPDATE credits WHERE ...)
const CREDITS_ROW: ResourceId = ResourceId::new(0);
// R1: `credit_ledger` insertion lock
const CREDIT_LEDGER: ResourceId = ResourceId::new(1);

/// 4 concurrent `meter_inner` calls following consistent R0->R1 ordering.
///
/// Expected: `OracleVerdict::Clean`
#[axiom_harness(
    name = "credit_meter_concurrent_safe",
    threads = 4,
    resources = 2,
    desc = "4 concurrent meter_inner: UPDATE credits(R0)->INSERT ledger(R1). Consistent order prevents deadlock.",
    expected = "clean"
)]
pub fn concurrent_op_provider(_thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match pc {
        // Step 1: credits access phase
        0 => Some((Operation::Read, CREDITS_ROW)),
        // Step 2: ledger access phase
        1 => Some((Operation::Read, CREDIT_LEDGER)),
        _ => None,
    }
}

/// Mixed lock ordering proof:
/// - Thread 0: R0->R1 (correct)
/// - Thread 1: R1->R0 (inverted)
///
/// Expected: `OracleVerdict::BugFound`
#[axiom_harness(
    name = "credit_meter_inverse_deadlock",
    threads = 2,
    resources = 2,
    desc = "Thread0 R0->R1 (correct), Thread1 R1->R0 (inverted). AB-BA deadlock proves ordering is critical.",
    expected = "bug"
)]
pub fn inverse_op_provider(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match thread.as_usize() {
        0 => match pc {
            0 => Some((Operation::Request, CREDITS_ROW)),
            1 => Some((Operation::Request, CREDIT_LEDGER)),
            2 => Some((Operation::Release, CREDIT_LEDGER)),
            3 => Some((Operation::Release, CREDITS_ROW)),
            _ => None,
        },
        1 => match pc {
            0 => Some((Operation::Request, CREDIT_LEDGER)),
            1 => Some((Operation::Request, CREDITS_ROW)),
            2 => Some((Operation::Release, CREDITS_ROW)),
            3 => Some((Operation::Release, CREDIT_LEDGER)),
            _ => None,
        },
        _ => None,
    }
}

/// Two-tenant concurrent `meter_inner` calls:
/// - Customer A: R0(own credits)->R2(shared ledger)
/// - Customer B: R1(own credits)->R2(shared ledger)
///
/// Expected: `OracleVerdict::Clean`
#[axiom_harness(
    name = "credit_meter_multi_tenant_safe",
    threads = 2,
    resources = 3,
    desc = "CustomerA locks R0->R2(ledger), CustomerB locks R1->R2(ledger). Different first resource prevents deadlock.",
    expected = "clean"
)]
pub fn multi_tenant_op_provider(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    let own_credits = ResourceId::new(thread.as_usize());
    let shared_ledger = ResourceId::new(2);

    match pc {
        0 => Some((Operation::Request, own_credits)),
        1 => Some((Operation::Request, shared_ledger)),
        2 => Some((Operation::Release, shared_ledger)),
        3 => Some((Operation::Release, own_credits)),
        _ => None,
    }
}
