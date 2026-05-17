//! Liveness / fairness harnesses — starvation and priority inversion scenarios.

use laplace_core::domain::resource::{ResourceId, ThreadId};
use laplace_dpor::Operation;
use laplace_macro::axiom_harness;

/// Greedy thread (T0) repeatedly requests R0 without releasing, eventually
/// causing self-contention: T0 holds R0 and then tries to acquire it again,
/// blocking itself.  Victim threads T1 and T2 are also blocked on T0-held R0.
/// All three threads end up blocked → deadlock.
///
/// Expected: `OracleVerdict::BugFound`.
#[axiom_harness(
    name = "resource_starvation_greedy",
    threads = 3,
    resources = 1,
    desc = "Greedy thread starves victim",
    expected = "bug"
)]
pub fn greedy_op_provider(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match thread.as_usize() {
        0 => {
            // Greedy: keeps requesting R0 without releasing — self-blocks after first
            // acquisition, preventing victims from ever making progress
            if pc < 3 {
                Some((Operation::Request, ResourceId::new(0)))
            } else {
                None
            }
        }
        1 | 2 => {
            // Victims: try to acquire R0, permanently blocked once T0 holds it
            match pc {
                0 => Some((Operation::Request, ResourceId::new(0))),
                1 => Some((Operation::Release, ResourceId::new(0))),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Thread 0 uses R0 exclusively; Thread 1 uses R1 exclusively.
/// No shared resources → no contention → no starvation.
///
/// Expected: `OracleVerdict::Clean`.
#[axiom_harness(
    name = "resource_fair_independent",
    threads = 2,
    resources = 2,
    desc = "Fair independent execution"
)]
pub fn fair_op_provider(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    // Each thread owns its own resource — guaranteed non-interfering
    let resource = ResourceId::new(thread.as_usize());
    match pc {
        0 => Some((Operation::Request, resource)),
        1 => Some((Operation::Release, resource)),
        _ => None,
    }
}

/// Classic priority inversion:
///
/// - Thread 0 (High):   Request R0 → Request R1 → Release R1 → Release R0
/// - Thread 1 (Low):    Request R1 → Request R0 → Release R0 → Release R1
/// - Thread 2 (Medium): Loops on R0 (5 iterations)
///
/// Medium-priority Thread 2 can continuously hold R0, preventing high-priority
/// Thread 0 from acquiring it while low-priority Thread 1 holds R1 — a textbook
/// priority inversion leading to potential deadlock.
///
/// Expected: `OracleVerdict::BugFound`.
#[axiom_harness(
    name = "resource_priority_inversion",
    threads = 3,
    resources = 2,
    desc = "Priority inversion scenario",
    expected = "bug"
)]
pub fn priority_op_provider(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match thread.as_usize() {
        0 => match pc {
            // High priority: needs R0 then R1
            0 => Some((Operation::Request, ResourceId::new(0))),
            1 => Some((Operation::Request, ResourceId::new(1))),
            2 => Some((Operation::Release, ResourceId::new(1))),
            3 => Some((Operation::Release, ResourceId::new(0))),
            _ => None,
        },
        1 => match pc {
            // Low priority: needs R1 then R0 (reverse order)
            0 => Some((Operation::Request, ResourceId::new(1))),
            1 => Some((Operation::Request, ResourceId::new(0))),
            2 => Some((Operation::Release, ResourceId::new(0))),
            3 => Some((Operation::Release, ResourceId::new(1))),
            _ => None,
        },
        2 => {
            // Medium priority: 5 loops on R0, starving Thread 0
            if pc < 10 {
                match pc % 2 {
                    0 => Some((Operation::Request, ResourceId::new(0))),
                    _ => Some((Operation::Release, ResourceId::new(0))),
                }
            } else {
                None
            }
        }
        _ => None,
    }
}
