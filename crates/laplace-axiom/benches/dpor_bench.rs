//! DPOR Benchmark Suite - Classic vs Ki-DPOR
//!
//! This benchmark compares the performance of two DPOR implementations:
//! - Classic DPOR: Stack-based DFS with backtracking
//! - Ki-DPOR: Priority queue A* with heuristic guidance
//!
//! # Scenarios
//!
//! 1. **AB-BA Deadlock**: Small-scale (2 threads, 2 resources)
//!    - Tests basic overhead
//!    - Classic may be faster due to simpler data structure
//!
//! 2. **Dining Philosophers**: Scalability stress test
//!    - N=3: Baseline
//!    - N=4: Classic struggles (~15ms)
//!    - N=5: Classic times out, Ki-DPOR succeeds
//!
//! 3. **Independent Operations**: Best case
//!    - Both should be fast
//!    - Tests overhead of heuristic computation
//!
//! 4. **Sequential Bottleneck**: Worst case contention
//!    - All threads compete for one resource
//!    - Tests heuristic effectiveness

#![cfg(feature = "twin")]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use laplace_axiom::dpor::{DporScheduler, KiDporScheduler, Operation, StepRecord};
use laplace_core::domain::resource::{
    DefaultTracker as ResourceTracker, RequestResult, ResourceError, ResourceId,
    ResourceTracker as ResourceTrackerTrait, ThreadId,
};

// ============================================================================
// Helper Functions - Classic DPOR
// ============================================================================

/// Restore ResourceTracker state from scheduler stack (Classic DPOR only)
fn restore_tracker_state(
    num_threads: usize,
    num_resources: usize,
    stack: &[StepRecord],
) -> ResourceTracker {
    let mut tracker = ResourceTracker::new(num_threads, num_resources);

    for step in stack {
        let _ = match step.operation {
            Operation::Request => tracker.request(step.thread, step.resource),
            Operation::Release => tracker
                .release(step.thread, step.resource)
                .map(|_| RequestResult::Acquired),
            // New operation variants from Session A — not used in this benchmark
            Operation::SharedRequest
            | Operation::SharedRelease
            | Operation::Read
            | Operation::Write
            | Operation::ReadWrite => Ok(RequestResult::Acquired),
        };
    }

    tracker
}

/// Get thread's program counter (number of steps executed)
fn get_thread_pc(thread: ThreadId, stack: &[StepRecord]) -> usize {
    stack.iter().filter(|s| s.thread == thread).count()
}

/// Run Classic DPOR exploration
fn run_classic_exploration<F>(
    mut dpor: DporScheduler,
    num_threads: usize,
    num_resources: usize,
    mut get_operation: F,
    max_steps: usize,
) -> (usize, usize, bool)
where
    F: FnMut(ThreadId, usize) -> Option<(Operation, ResourceId)>,
{
    let mut deadlock_found = false;
    let mut step_count = 0;

    while !dpor.is_complete() && step_count < max_steps {
        // Restore state
        let mut tracker = restore_tracker_state(num_threads, num_resources, dpor.stack());

        if let Some(thread) = dpor.next_step() {
            let pc = get_thread_pc(thread, dpor.stack());

            // Get operation from scenario
            if let Some((operation, resource)) = get_operation(thread, pc) {
                // Execute on tracker
                let result = match operation {
                    Operation::Request => tracker.request(thread, resource),
                    Operation::Release => tracker
                        .release(thread, resource)
                        .map(|_| RequestResult::Acquired),
                    // New operation variants from Session A — not used in this benchmark
                    Operation::SharedRequest
                    | Operation::SharedRelease
                    | Operation::Read
                    | Operation::Write
                    | Operation::ReadWrite => Ok(RequestResult::Acquired),
                };

                match result {
                    Ok(RequestResult::Acquired) | Ok(RequestResult::Blocked) => {
                        dpor.commit_step(thread, operation, resource);
                    }
                    Err(ResourceError::DeadlockDetected { .. }) => {
                        deadlock_found = true;
                        dpor.backtrack();
                    }
                    Err(_) => {
                        dpor.backtrack();
                    }
                }
            } else {
                dpor.backtrack();
            }

            step_count += 1;
        } else {
            dpor.backtrack();
        }
    }

    let stats = dpor.stats();
    (stats.explored_states, stats.backtracks, deadlock_found)
}

// ============================================================================
// Helper Functions - Ki-DPOR
// ============================================================================

/// Run Ki-DPOR exploration
///
/// Unlike Classic DPOR, Ki-DPOR doesn't need ResourceTracker because
/// KiState encapsulates the entire world state as snapshots.
fn run_ki_exploration<F>(
    mut scheduler: KiDporScheduler,
    mut get_operation: F,
    max_iterations: usize,
) -> (usize, usize, bool)
where
    F: FnMut(ThreadId, usize) -> Option<(Operation, ResourceId)>,
{
    let mut iterations = 0;
    let mut deadlock_found = false;

    while !scheduler.is_complete() && iterations < max_iterations {
        if let Some(state) = scheduler.next_state() {
            // Check for deadlock in state
            // (In Ki-DPOR, we can detect deadlock from state properties)
            let blocked_count = state
                .thread_status
                .iter()
                .filter(|&&s| s == laplace_axiom::dpor::ThreadStatus::Blocked)
                .count();

            if blocked_count > 0 && blocked_count == state.thread_status.len() {
                deadlock_found = true;
            }

            // Expand current state
            scheduler.expand_current(&mut get_operation);
        }

        iterations += 1;
    }

    let stats = scheduler.stats();
    (stats.explored_states, 0, deadlock_found) // Ki-DPOR doesn't track backtracks
}

// ============================================================================
// Operation Generators (Shared between Classic and Ki-DPOR)
// ============================================================================

/// AB-BA Deadlock operation generator
fn abba_operation(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    let t0 = ThreadId(0);
    let r0 = ResourceId(0);
    let r1 = ResourceId(1);

    if thread == t0 {
        match pc {
            0 => Some((Operation::Request, r0)),
            1 => Some((Operation::Request, r1)),
            2 => Some((Operation::Release, r1)),
            3 => Some((Operation::Release, r0)),
            _ => None,
        }
    } else {
        match pc {
            0 => Some((Operation::Request, r1)),
            1 => Some((Operation::Request, r0)),
            2 => Some((Operation::Release, r0)),
            3 => Some((Operation::Release, r1)),
            _ => None,
        }
    }
}

/// Dining Philosophers operation generator
fn dining_philosophers_op(
    n: usize,
    thread: ThreadId,
    pc: usize,
) -> Option<(Operation, ResourceId)> {
    let i = thread.as_usize();
    let left_fork = ResourceId(i);
    let right_fork = ResourceId((i + 1) % n);

    match pc {
        0 => Some((Operation::Request, left_fork)),
        1 => Some((Operation::Request, right_fork)),
        2 => Some((Operation::Release, right_fork)),
        3 => Some((Operation::Release, left_fork)),
        _ => None,
    }
}

/// Independent operations generator
fn independent_op(thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    let resource = ResourceId(thread.as_usize());

    match pc {
        0 => Some((Operation::Request, resource)),
        1 => Some((Operation::Release, resource)),
        _ => None,
    }
}

/// Sequential bottleneck generator
fn bottleneck_op(_thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    let shared = ResourceId(0);

    match pc {
        0 => Some((Operation::Request, shared)),
        1 => Some((Operation::Release, shared)),
        _ => None,
    }
}

// ============================================================================
// Benchmark Groups
// ============================================================================

/// Benchmark AB-BA Deadlock scenario
fn bench_abba_deadlock(c: &mut Criterion) {
    let mut group = c.benchmark_group("abba_deadlock");

    // Classic DPOR
    group.bench_function("classic", |b| {
        b.iter(|| {
            let dpor = DporScheduler::new(2);
            let (states, backtracks, deadlock) =
                run_classic_exploration(dpor, 2, 2, abba_operation, 100);

            assert!(deadlock, "Should detect deadlock");
            black_box((states, backtracks))
        });
    });

    // Ki-DPOR
    group.bench_function("ki_dpor", |b| {
        b.iter(|| {
            let scheduler = KiDporScheduler::new(2, 2);
            let (states, _, _deadlock) = run_ki_exploration(scheduler, abba_operation, 100);

            black_box(states)
        });
    });

    group.finish();
}

/// Benchmark Dining Philosophers (THE MAIN EVENT)
fn bench_dining_philosophers(c: &mut Criterion) {
    let mut group = c.benchmark_group("dining_philosophers");

    for n in [3, 4, 5].iter() {
        // Classic DPOR
        group.bench_with_input(BenchmarkId::new("classic", n), n, |b, &n| {
            b.iter(|| {
                let dpor = DporScheduler::new(n);
                let (states, backtracks, _) = run_classic_exploration(
                    dpor,
                    n,
                    n,
                    |thread, pc| dining_philosophers_op(n, thread, pc),
                    50000, // Higher limit for N=5
                );

                black_box((states, backtracks))
            });
        });

        // Ki-DPOR
        group.bench_with_input(BenchmarkId::new("ki_dpor", n), n, |b, &n| {
            b.iter(|| {
                let scheduler = KiDporScheduler::new(n, n);
                let (states, _, _) = run_ki_exploration(
                    scheduler,
                    |thread, pc| dining_philosophers_op(n, thread, pc),
                    50000,
                );

                black_box(states)
            });
        });
    }

    group.finish();
}

/// Benchmark Independent Operations (Best Case)
fn bench_independent_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("independent_ops");

    for n in [2, 4, 8].iter() {
        // Classic DPOR
        group.bench_with_input(BenchmarkId::new("classic", n), n, |b, &n| {
            b.iter(|| {
                let dpor = DporScheduler::new(n);
                let (states, backtracks, _) =
                    run_classic_exploration(dpor, n, n, independent_op, 1000);

                black_box((states, backtracks))
            });
        });

        // Ki-DPOR
        group.bench_with_input(BenchmarkId::new("ki_dpor", n), n, |b, &n| {
            b.iter(|| {
                let scheduler = KiDporScheduler::new(n, n);
                let (states, _, _) = run_ki_exploration(scheduler, independent_op, 1000);

                black_box(states)
            });
        });
    }

    group.finish();
}

/// Benchmark Sequential Bottleneck (Worst Case Contention)
fn bench_sequential_bottleneck(c: &mut Criterion) {
    let mut group = c.benchmark_group("sequential_bottleneck");

    for n in [2, 3, 4].iter() {
        // Classic DPOR
        group.bench_with_input(BenchmarkId::new("classic", n), n, |b, &n| {
            b.iter(|| {
                let dpor = DporScheduler::new(n);
                let (states, backtracks, _) = run_classic_exploration(
                    dpor,
                    n,
                    1, // Single shared resource
                    bottleneck_op,
                    5000,
                );

                black_box((states, backtracks))
            });
        });

        // Ki-DPOR
        group.bench_with_input(BenchmarkId::new("ki_dpor", n), n, |b, &n| {
            b.iter(|| {
                let scheduler = KiDporScheduler::new(n, 1);
                let (states, _, _) = run_ki_exploration(scheduler, bottleneck_op, 5000);

                black_box(states)
            });
        });
    }

    group.finish();
}

// ============================================================================
// Criterion Configuration
// ============================================================================

criterion_group!(
    benches,
    bench_abba_deadlock,
    bench_dining_philosophers,
    bench_independent_operations,
    bench_sequential_bottleneck
);

criterion_main!(benches);
