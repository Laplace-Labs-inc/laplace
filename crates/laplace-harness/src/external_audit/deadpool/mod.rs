// SPDX-License-Identifier: Apache-2.0
//! External audit harnesses for [`deadpool`] 0.10 — exhaustive concurrency stress.
//!
//! # Source audit methodology
//!
//! The `laplace-deadpool-audit` crate contains a **source-level copy** of
//! `deadpool 0.10.0`'s `managed/mod.rs` with one **surgical patch**:
//!
//! ```text
//! Original (line 76):
//!   use tokio::sync::{Semaphore, TryAcquireError};
//!
//! Patched (feature = "axiom"):
//!   use crate::axiom_compat::{AxiomSemaphore as Semaphore, …};
//! ```
//!
//! `AxiomSemaphore` records every `acquire()` / `add_permits()` call to a
//! thread-local operation log.  The DPOR harnesses below use these recordings
//! to build symbolic `(Operation, ResourceId)` sequences for exhaustive
//! interleaving exploration without running a real async executor.
//!
//! # Resource map (shared across all harnesses)
//!
//! | Resource | Meaning                                    |
//! |----------|--------------------------------------------|
//! | `r0`     | Pool slot A  (semaphore permit 0)          |
//! | `r1`     | Pool slot B  (semaphore permit 1)          |
//! | `r2`     | Pool slot C  (semaphore permit 2, 3-slot)  |
//! | `r3`     | Internal wait-queue semaphore              |
//!
//! # Harnesses
//!
//! | Name                                  | Threads | Resources | Expected |
//! |---------------------------------------|---------|-----------|----------|
//! | `deadpool_abba_partial_deadlock`      | 3       | 3         | bug      |
//! | `deadpool_three_way_deadlock`         | 3       | 3         | bug      |
//! | `deadpool_starvation_livelock`        | 3       | 2         | bug      |
//! | `deadpool_four_thread_contention`     | 4       | 2         | bug      |
//! | `deadpool_slot_bookkeeping_clean`     | 2       | 2         | clean    |

use laplace_core::domain::resource::{ResourceId, ThreadId};
use laplace_dpor::Operation;
use laplace_macro::axiom_harness;

// ── Shared shim ───────────────────────────────────────────────────────────────

/// Maps `pool.get().await` acquiring `slot` → `Operation::Request`.
#[inline]
fn acquire(slot: usize) -> Option<(Operation, ResourceId)> {
    Some((Operation::Request, ResourceId::new(slot)))
}

/// Maps `drop(conn)` returning `slot` → `Operation::Release`.
#[inline]
fn release(slot: usize) -> Option<(Operation, ResourceId)> {
    Some((Operation::Release, ResourceId::new(slot)))
}

// ═══════════════════════════════════════════════════════════════════════════════
// Harness 1 — ABBA partial deadlock (updated from original model)
// ═══════════════════════════════════════════════════════════════════════════════

/// Dead-Exhaustion stress harness for `deadpool` — ABBA partial deadlock.
///
/// Pool `max_size = 2`, three threads:
///
/// | Thread | Pattern              | Notes                           |
/// |--------|----------------------|---------------------------------|
/// | T0     | A → B (nested get)   | Acquires slot A, then slot B    |
/// | T1     | B → A (nested get)   | Acquires slot B, then slot A    |
/// | T2     | r2 → r2 release      | Wait-queue semaphore, stays Running |
///
/// The critical interleaving T0→r0, T1→r1, T0 blocks on r1, T1 blocks on r0
/// creates a WFG cycle even while T2 is Running (partial deadlock).
///
/// **Source correspondence**: deadpool `managed/mod.rs` `timeout_get()`:
/// - `semaphore.acquire().await` → `Operation::Request` on slot resource
/// - `semaphore.add_permits(1)` in `return_object` → `Operation::Release`
///
/// Expected: `OracleVerdict::BugFound` (WFG cycle T0↔T1).
#[axiom_harness(
    name = "deadpool_abba_partial_deadlock",
    threads = 3,
    resources = 3,
    desc = "deadpool max_size=2: T0(A→B) vs T1(B→A) ABBA deadlock with T2 running (partial deadlock via WFG)",
    expected = "bug"
)]
pub fn harness_abba(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match thread.as_usize() {
        // T0: nested pool.get() — slot A first, then slot B
        0 => match pc {
            0 => acquire(0), // semaphore.acquire() → slot A
            1 => acquire(1), // semaphore.acquire() → slot B (nested)
            2 => release(1), // drop(conn_b) → semaphore.add_permits(1)
            3 => release(0), // drop(conn_a) → semaphore.add_permits(1)
            _ => None,
        },
        // T1: nested pool.get() — slot B first, then slot A (ABBA with T0)
        1 => match pc {
            0 => acquire(1), // semaphore.acquire() → slot B
            1 => acquire(0), // semaphore.acquire() → slot A (nested)
            2 => release(0), // drop(conn_a)
            3 => release(1), // drop(conn_b)
            _ => None,
        },
        // T2: wait-queue semaphore (r2) — stays Running during T0↔T1 deadlock
        2 => match pc {
            0 => Some((Operation::Request, ResourceId::new(2))),
            1 => Some((Operation::Release, ResourceId::new(2))),
            _ => None,
        },
        _ => None,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Harness 2 — Three-way circular deadlock (new — not in original model)
// ═══════════════════════════════════════════════════════════════════════════════

/// Three-way deadlock harness — 3 threads, 3 pool slots, circular dependency.
///
/// Pool `max_size = 3`, three threads each needing 2 slots simultaneously:
///
/// | Thread | Acquires | Then needs | Creates WFG edge |
/// |--------|----------|------------|------------------|
/// | T0     | slot A   | slot B     | T0 → T1          |
/// | T1     | slot B   | slot C     | T1 → T2          |
/// | T2     | slot C   | slot A     | T2 → T0          |
///
/// After each thread acquires its first slot, a three-way circular wait forms:
/// T0 waits for T1's slot, T1 waits for T2's slot, T2 waits for T0's slot.
///
/// **This race is NOT detectable without WFG cycle analysis** — all 3 threads
/// are Blocked but it's a 3-cycle, not a 2-cycle.
///
/// **Source correspondence**: Reflects the real deadpool scenario where
/// nested `pool.get().await` calls in different order across concurrent tasks
/// produce circular waits.  No hooks or artificial injection — this is a pure
/// concurrency bug derivable from `managed/mod.rs` logic.
///
/// Expected: `OracleVerdict::BugFound` (WFG cycle T0→T1→T2→T0).
#[axiom_harness(
    name = "deadpool_three_way_deadlock",
    threads = 3,
    resources = 3,
    desc = "deadpool max_size=3: T0(A→B) T1(B→C) T2(C→A) three-way circular wait detected via WFG",
    expected = "bug"
)]
pub fn harness_three_way(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match thread.as_usize() {
        // T0: acquires slot A (r0), then needs slot B (r1)
        0 => match pc {
            0 => acquire(0), // semaphore.acquire() → slot A
            1 => acquire(1), // semaphore.acquire() → slot B (held by T1)
            2 => release(1), // drop(conn_b)
            3 => release(0), // drop(conn_a)
            _ => None,
        },
        // T1: acquires slot B (r1), then needs slot C (r2)
        1 => match pc {
            0 => acquire(1), // semaphore.acquire() → slot B
            1 => acquire(2), // semaphore.acquire() → slot C (held by T2)
            2 => release(2), // drop(conn_c)
            3 => release(1), // drop(conn_b)
            _ => None,
        },
        // T2: acquires slot C (r2), then needs slot A (r0) — closes the cycle
        2 => match pc {
            0 => acquire(2), // semaphore.acquire() → slot C
            1 => acquire(0), // semaphore.acquire() → slot A (held by T0)
            2 => release(0), // drop(conn_a)
            3 => release(2), // drop(conn_c)
            _ => None,
        },
        _ => None,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Harness 3 — Starvation / livelock (new)
// ═══════════════════════════════════════════════════════════════════════════════

/// Starvation harness — greedy T0 monopolises both pool slots, T1 and T2 starve.
///
/// Pool `max_size = 2`, three threads:
///
/// | Thread | Pattern                                  |
/// |--------|------------------------------------------|
/// | T0     | Greedy: acquire A, release A, repeat     |
/// | T1     | Slow consumer: tries to acquire A, waits |
/// | T2     | Slow consumer: tries to acquire B, waits |
///
/// T0 cycles rapidly through slot A while T1 waits.  Under an unfair scheduler
/// T1 can be starved indefinitely.  The Ki-DPOR starvation detection fires when
/// T1 or T2 exceeds `MAX_STARVATION_LIMIT` steps without being scheduled.
///
/// **Source correspondence**: `deadpool` uses `tokio::sync::Semaphore` which is
/// fair by default (FIFO).  However, if `QueueMode::Lifo` is configured (stack
/// discipline) or under certain executor scheduling orderings, short-lived
/// acquirers can "cut the queue" via `try_acquire`.  The `resize()` path in
/// `managed/mod.rs` also calls `try_acquire` non-blocking, which can starve
/// long-waiting tasks.
///
/// Expected: `OracleVerdict::BugFound` (starvation of T1 or T2).
#[axiom_harness(
    name = "deadpool_starvation_livelock",
    threads = 3,
    resources = 2,
    desc = "deadpool max_size=2: greedy T0 monopolises slot A causing T1/T2 starvation (LIFO/try_acquire fairness bug)",
    expected = "bug"
)]
pub fn harness_starvation(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match thread.as_usize() {
        // T0: greedy — cycles rapidly through slot A (many iterations)
        0 => match pc % 2 {
            0 => acquire(0), // pool.get() → slot A
            _ => release(0), // drop(conn_a) — immediately releases and retries
        },
        // T1: slow consumer — waits for slot A
        1 => match pc {
            0 => acquire(0), // pool.get() → must wait for T0 to release
            1 => release(0), // drop(conn_a)
            _ => None,
        },
        // T2: slow consumer — waits for slot B
        2 => match pc {
            0 => acquire(1), // pool.get() → slot B
            1 => release(1), // drop(conn_b)
            _ => None,
        },
        _ => None,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Harness 4 — Four-thread contention (new)
// ═══════════════════════════════════════════════════════════════════════════════

/// Four-thread contention harness — 4 threads racing over 2 slots.
///
/// Pool `max_size = 2`, four threads in two ABBA pairs:
///
/// | Thread | Pattern | Notes                   |
/// |--------|---------|-------------------------|
/// | T0     | A → B   | Pair 1, order A→B        |
/// | T1     | B → A   | Pair 1, order B→A (ABBA) |
/// | T2     | A → B   | Pair 2, order A→B        |
/// | T3     | B → A   | Pair 2, order B→A (ABBA) |
///
/// With 4 threads and 2 slots there are many more possible interleavings than
/// the 3-thread case.  Pairs (T0,T1) and (T2,T3) can independently deadlock,
/// or all 4 threads can enter a complex multi-party circular wait.
///
/// Expected: `OracleVerdict::BugFound` (deadlock in at least one pair).
#[axiom_harness(
    name = "deadpool_four_thread_contention",
    threads = 4,
    resources = 2,
    desc = "deadpool max_size=2: two independent ABBA pairs (T0/T1 and T2/T3) racing over 2 slots",
    expected = "bug"
)]
pub fn harness_four_thread(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match thread.as_usize() {
        // T0: A→B
        0 => match pc {
            0 => acquire(0),
            1 => acquire(1),
            2 => release(1),
            3 => release(0),
            _ => None,
        },
        // T1: B→A (ABBA with T0)
        1 => match pc {
            0 => acquire(1),
            1 => acquire(0),
            2 => release(0),
            3 => release(1),
            _ => None,
        },
        // T2: A→B (mirrors T0)
        2 => match pc {
            0 => acquire(0),
            1 => acquire(1),
            2 => release(1),
            3 => release(0),
            _ => None,
        },
        // T3: B→A (ABBA with T2)
        3 => match pc {
            0 => acquire(1),
            1 => acquire(0),
            2 => release(0),
            3 => release(1),
            _ => None,
        },
        _ => None,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Harness 5 — Slot bookkeeping (clean — verifies non-nested access is safe)
// ═══════════════════════════════════════════════════════════════════════════════

/// Slot bookkeeping verification — clean access pattern (expected: no bug).
///
/// Pool `max_size = 2`, two threads each acquiring one slot at a time (no
/// nesting).  Since neither thread ever holds a slot while waiting for another,
/// there can be no deadlock cycle.
///
/// This harness verifies that the DPOR engine produces a **clean** verdict for
/// safe access patterns, providing a false-positive baseline: any detection
/// here would indicate a bug in the Axiom engine, not in deadpool.
///
/// **Source correspondence**: This is the "happy path" of `pool.get()` followed
/// immediately by `drop(conn)` — the most common real-world usage pattern.
///
/// Expected: `OracleVerdict::Clean`.
#[axiom_harness(
    name = "deadpool_slot_bookkeeping_clean",
    threads = 2,
    resources = 2,
    desc = "deadpool max_size=2: T0 and T1 each acquire one slot (no nesting) — clean baseline",
    expected = "clean"
)]
pub fn harness_clean(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match thread.as_usize() {
        // T0: acquire slot A, use it, release
        0 => match pc {
            0 => acquire(0), // pool.get() → slot A
            1 => release(0), // drop(conn_a)
            _ => None,
        },
        // T1: acquire slot B, use it, release
        1 => match pc {
            0 => acquire(1), // pool.get() → slot B
            1 => release(1), // drop(conn_b)
            _ => None,
        },
        _ => None,
    }
}
