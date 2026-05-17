//! DPOR Integration Benchmark — Cross-Transfer Deadlock Detection
//!
//! Models the classic AB-BA (cross-transfer) deadlock:
//!
//! ```text
//! Task A (ThreadId 0):  pc=0 → Request ResourceId(0)  [acquire X]
//!                       pc=1 → Request ResourceId(1)  [request Y — blocked if B holds Y]
//!
//! Task B (ThreadId 1):  pc=0 → Request ResourceId(1)  [acquire Y]
//!                       pc=1 → Request ResourceId(0)  [request X — blocked if A holds X]
//! ```
//!
//! When both tasks advance past pc=0 before either releases, they form a cycle:
//! A waits for Y (held by B) and B waits for X (held by A) → deadlock.
//!
//! # Benchmark Groups
//!
//! - `dpor/pruning_efficiency` — complete Ki-DPOR exploration of the deadlock
//!   scenario; measures scheduler throughput and the explored-state count after
//!   partial-order pruning.
//!
//! - `dpor/evasion_latency` — end-to-end latency from fresh scheduler
//!   construction through deadlock detection via `liveness_violation()`.
//!   Uses `BatchSize::SmallInput` so scheduler setup is excluded from the
//!   measured window; targets ~2.5 µs per iteration.

#![cfg(feature = "twin")]

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use laplace_axiom::dpor::{KiDporScheduler, LivenessViolation, Operation};
use laplace_core::domain::resource::{ResourceId, ThreadId};
use std::time::Duration;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Cross-Transfer Deadlock Scenario
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Operation generator for the cross-transfer deadlock program.
///
/// - Thread 0: Request(X=0) → Request(Y=1)
/// - Thread 1: Request(Y=1) → Request(X=0)
fn cross_transfer_op(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match (thread, pc) {
        (ThreadId(0), 0) => Some((Operation::Request, ResourceId(0))),
        (ThreadId(0), 1) => Some((Operation::Request, ResourceId(1))),
        (ThreadId(1), 0) => Some((Operation::Request, ResourceId(1))),
        (ThreadId(1), 1) => Some((Operation::Request, ResourceId(0))),
        _ => None,
    }
}

/// Run a complete Ki-DPOR exploration of the cross-transfer scenario.
///
/// Returns `(explored_states, deadlock_detected)`.
fn run_cross_transfer(max_iterations: usize) -> (usize, bool) {
    let mut scheduler = KiDporScheduler::new(2, 2);
    let mut iterations = 0;

    while !scheduler.is_complete() && iterations < max_iterations {
        if scheduler.next_state().is_some() {
            scheduler.expand_current(cross_transfer_op);
        }
        iterations += 1;
    }

    let deadlock = matches!(
        scheduler.liveness_violation(),
        Some(LivenessViolation::Deadlock { .. })
    );

    (scheduler.explored_count(), deadlock)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// A. dpor/pruning_efficiency — exploration throughput after DPOR pruning
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_pruning_efficiency(c: &mut Criterion) {
    let mut group = c.benchmark_group("dpor/pruning_efficiency");
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(3));
    group.throughput(Throughput::Elements(1));

    // One complete exploration of the cross-transfer deadlock scenario.
    // The explored_count() reflects the pruned search space; assert ensures
    // the deadlock is actually found so the measurement is representative.
    group.bench_function("cross_transfer/ki_dpor", |b| {
        b.iter(|| {
            let (explored, deadlock) = run_cross_transfer(1_000);
            assert!(deadlock, "Ki-DPOR must detect the cross-transfer deadlock");
            std::hint::black_box((explored, deadlock))
        })
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// B. dpor/evasion_latency — end-to-end deadlock detection latency
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_evasion_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("dpor/evasion_latency");
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(3));

    // Measure the exploration loop that finds the deadlock.
    //
    // setup:   construct a fresh KiDporScheduler (excluded from timing)
    // routine: run the exploration loop until is_complete(), return whether
    //          a Deadlock liveness violation was detected.
    //
    // BatchSize::SmallInput ensures Criterion prepares a fresh scheduler per
    // sample batch without including construction cost in the measured window.
    // Target: ~2.5 µs per iteration for the 2-thread × 2-resource scenario.
    group.bench_function("cross_transfer/detect", |b| {
        b.iter_batched(
            || KiDporScheduler::new(2, 2),
            |mut scheduler| {
                let mut iterations = 0usize;
                while !scheduler.is_complete() && iterations < 1_000 {
                    if scheduler.next_state().is_some() {
                        scheduler.expand_current(cross_transfer_op);
                    }
                    iterations += 1;
                }
                std::hint::black_box(matches!(
                    scheduler.liveness_violation(),
                    Some(LivenessViolation::Deadlock { .. })
                ))
            },
            BatchSize::SmallInput,
        )
    });

    group.finish();
}

criterion_group!(
    dpor_integration_benches,
    bench_pruning_efficiency,
    bench_evasion_latency
);
criterion_main!(dpor_integration_benches);
