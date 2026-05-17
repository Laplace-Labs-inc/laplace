//! Simulation Integration Benchmarks
//!
//! Measures the performance of `domain::simulation` module components:
//!
//! - `simulation/init`       — Builder construction: `ProductionSimulatorBuilder`,
//!                             `VerificationSimulatorBuilder`, with and without tracer.
//! - `simulation/step`       — Single `step()` call with 1 pre-queued event.
//!                             Compares MemoryWriteSync, MemoryFence, and passthrough
//!                             (`Test`) payloads across production and verification engines.
//! - `simulation/batch`      — `run_until_idle()` throughput with 10 / 100 / 1000 events
//!                             for production; 4 events for verification (backend limit).
//! - `simulation/reset`      — `reset()` cost on a fresh vs. post-run simulator.
//! - `simulation/dispatcher` — Direct `EventDispatcher` construction and `process_event` /
//!                             `process_all` calls, isolating the ZST routing overhead.
//!
//! **Zero-Implementation Rule**: all benchmarks call only the existing public API.
//! No custom scheduling logic, no OS thread spawning, no external event loops.
//!
//! **Compare Engines Rule**: every meaningful workload is benchmarked on both
//! `ProductionSimulator` (heap-backed, DashMap) and `VerificationSimulator`
//! (stack-backed, `RefCell` + fixed arrays) to expose backend cost differences.

use criterion::{black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use laplace_axiom::simulation::{
    EventDispatcher, ProductionSimulator, ProductionSimulatorBuilder, VerificationSimulator,
    VerificationSimulatorBuilder,
};
use laplace_core::domain::memory::{Address, CoreId, Value};
use laplace_core::domain::time::EventPayload;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Setup helpers — called inside iter_batched routines
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Build a `ProductionSimulator` with exactly `n` MemoryWriteSync events pre-queued.
///
/// Buffer capacity per core is `ceil(n / 4)` so every `write()` succeeds even
/// when all events target the same core.  Writes are round-robined across 4 cores
/// and up to 256 unique addresses.
fn prod_sim_with_writes(n: usize) -> ProductionSimulator {
    let cores = 4usize;
    let buf = (n.max(1) + cores - 1) / cores; // ceil(n / cores)
    let mut sim = ProductionSimulatorBuilder::new()
        .num_cores(cores)
        .buffer_size(buf)
        .build();
    for i in 0..n {
        sim.memory_mut()
            .write(
                CoreId::new(i % cores),
                Address::new(i % 256),
                Value::new(i as u64),
            )
            .unwrap();
    }
    sim
}

/// Build a `ProductionSimulator` with 2 write events buffered and a `MemoryFence`
/// event scheduled at virtual time 0 — **before** the MemoryWriteSync events at
/// time 1.  This ensures the fence step actually drains a non-empty store buffer,
/// giving a realistic measurement of the flush loop.
fn prod_sim_with_early_fence() -> ProductionSimulator {
    let mut sim = ProductionSimulatorBuilder::new()
        .num_cores(4)
        .buffer_size(2)
        .build();
    // Two writes fill core 0's store buffer and schedule WriteSync events at T=1.
    sim.memory_mut()
        .write(CoreId::new(0), Address::new(0), Value::new(1))
        .unwrap();
    sim.memory_mut()
        .write(CoreId::new(0), Address::new(1), Value::new(2))
        .unwrap();
    // Fence at delay 0 → scheduled at T=0 < T=1 → fires first.
    // Dispatcher arm: while buffer_len(core0) > 0 { flush_one } × 2.
    sim.memory_mut().clock_mut().schedule(
        0u64,
        EventPayload::MemoryFence {
            core: CoreId::new(0),
        },
    );
    sim
}

/// Build a `ProductionSimulator` with 1 passthrough (`Test`) event in the clock queue.
///
/// The dispatcher matches `Test(_)` → no-op body, proving zero-cost event routing.
fn prod_sim_with_passthrough() -> ProductionSimulator {
    let mut sim = ProductionSimulatorBuilder::new()
        .num_cores(4)
        .buffer_size(2)
        .build();
    sim.memory_mut()
        .clock_mut()
        .schedule(1u64, EventPayload::Test(black_box(42u64)));
    sim
}

/// Build a `VerificationSimulator` with `n` MemoryWriteSync events pre-queued.
///
/// `n` must be ≤ 4 (VerificationBackend hard limit: 2 cores × MAX_BUFFER_ENTRIES=2).
fn verif_sim_with_writes(n: usize) -> VerificationSimulator {
    debug_assert!(
        n <= 4,
        "VerificationBackend: max 2 cores × 2 buffer entries = 4"
    );
    let mut sim = VerificationSimulatorBuilder::build();
    for i in 0..n {
        sim.memory_mut()
            .write(
                CoreId::new(i % 2),  // 2 cores
                Address::new(i % 4), // 4 addressable locations
                Value::new(i as u64),
            )
            .unwrap();
    }
    sim
}

/// Build a `VerificationSimulator` with 2 writes buffered and an early `MemoryFence`.
///
/// Same early-fence trick as `prod_sim_with_early_fence` but using the verification
/// backend (RefCell + fixed arrays, no heap, no locking).
fn verif_sim_with_early_fence() -> VerificationSimulator {
    let mut sim = VerificationSimulatorBuilder::build();
    sim.memory_mut()
        .write(CoreId::new(0), Address::new(0), Value::new(1))
        .unwrap();
    sim.memory_mut()
        .write(CoreId::new(0), Address::new(1), Value::new(2))
        .unwrap();
    sim.memory_mut().clock_mut().schedule(
        0u64,
        EventPayload::MemoryFence {
            core: CoreId::new(0),
        },
    );
    sim
}

/// Build a `VerificationSimulator` with 1 passthrough (`Test`) event.
fn verif_sim_with_passthrough() -> VerificationSimulator {
    let mut sim = VerificationSimulatorBuilder::build();
    sim.memory_mut()
        .clock_mut()
        .schedule(1u64, EventPayload::Test(black_box(0u64)));
    sim
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 1: Initialization
//
// Measures builder construction cost — DashMap + VecDeque allocation for production
// vs. RefCell + fixed arrays for verification.
//
// ProductionSimulatorBuilder:
//   .build()             — allocates heap-backed memory backend + clock
//   .build_with_tracer() — same + TraceEngine (Vec-backed event log)
//
// VerificationSimulatorBuilder (all stack-allocated, zero heap):
//   ::build()            — RefCell<[BufferState; 2]> + RefCell<[u64; 4]>
//   ::build_with_tracer() — same + VerificationTracer (array-backed)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_simulation_init(c: &mut Criterion) {
    let mut group = c.benchmark_group("simulation/init");

    // Production build — parameterised by core count (shows DashMap/VecDeque scaling)
    for &cores in &[4usize, 8] {
        group.bench_with_input(
            BenchmarkId::new("build/prod", cores),
            &cores,
            |b, &cores| {
                b.iter(|| {
                    black_box(
                        ProductionSimulatorBuilder::new()
                            .num_cores(cores)
                            .buffer_size(2)
                            .build(),
                    )
                })
            },
        );
    }

    // Production build_with_tracer — adds TraceEngine (Vec allocation)
    group.bench_function("build_with_tracer/prod", |b| {
        b.iter(|| {
            black_box(
                ProductionSimulatorBuilder::new()
                    .num_cores(4)
                    .buffer_size(2)
                    .enable_tracing(true)
                    .max_traced_events(1000)
                    .build_with_tracer(),
            )
        })
    });

    // Verification build — all stack-allocated, should be near zero heap cost
    group.bench_function("build/verif", |b| {
        b.iter(|| black_box(VerificationSimulatorBuilder::build()))
    });

    // Verification build_with_tracer — adds VerificationTracer (fixed array on stack)
    group.bench_function("build_with_tracer/verif", |b| {
        b.iter(|| black_box(VerificationSimulatorBuilder::build_with_tracer()))
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 2: Single Step Overhead
//
// Measures one `simulator.step()` call with a pre-queued event.
// iter_batched ensures a fresh simulator with exactly the right events for each
// iteration so the measured window is: clock.tick() + match + handler.
//
// Three payload paths through EventDispatcher::dispatch_event:
//
//   MemoryWriteSync → flush_one (buffer pop + main-memory write)
//   MemoryFence     → while buffer_len > 0 { flush_one }  (early fence: 2 iterations)
//   Test(_)         → {} (no-op body — pure match routing cost)
//
// Production backend (parking_lot::RwLock, DashMap shards) vs.
// Verification backend (RefCell + fixed arrays, no heap).
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_simulation_step(c: &mut Criterion) {
    let mut group = c.benchmark_group("simulation/step");

    // ── ProductionSimulator ──────────────────────────────────────────────────

    // MemoryWriteSync dispatch: clock.tick() + flush_one (buffer pop + main write)
    group.bench_function("step/prod/write", |b| {
        b.iter_batched(
            || prod_sim_with_writes(1),
            |mut sim| black_box(sim.step()),
            BatchSize::SmallInput,
        )
    });

    // MemoryFence dispatch: clock.tick() + while(2 > 0) { flush_one } × 2
    // (Fence is scheduled before WriteSync events so buffer is non-empty at dispatch time)
    group.bench_function("step/prod/fence", |b| {
        b.iter_batched(
            || prod_sim_with_early_fence(),
            |mut sim| black_box(sim.step()),
            BatchSize::SmallInput,
        )
    });

    // Test(_) dispatch: clock.tick() + match arm → empty body → return true
    // Proves zero-cost "assembly-level" routing overhead for passthrough events
    group.bench_function("step/prod/passthrough", |b| {
        b.iter_batched(
            || prod_sim_with_passthrough(),
            |mut sim| black_box(sim.step()),
            BatchSize::SmallInput,
        )
    });

    // ── VerificationSimulator ────────────────────────────────────────────────

    group.bench_function("step/verif/write", |b| {
        b.iter_batched(
            || verif_sim_with_writes(1),
            |mut sim| black_box(sim.step()),
            BatchSize::SmallInput,
        )
    });

    group.bench_function("step/verif/fence", |b| {
        b.iter_batched(
            || verif_sim_with_early_fence(),
            |mut sim| black_box(sim.step()),
            BatchSize::SmallInput,
        )
    });

    group.bench_function("step/verif/passthrough", |b| {
        b.iter_batched(
            || verif_sim_with_passthrough(),
            |mut sim| black_box(sim.step()),
            BatchSize::SmallInput,
        )
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 3: Batch Throughput — run_until_idle
//
// Pre-loads N write events then drains the entire queue in one call to
// `run_until_idle()` (delegates to EventDispatcher::process_all with MAX=1000).
//
// Production:    N ∈ {10, 100, 1000}  — heap-backed, scalable buffer sizes
// Verification:  N = 4               — hard limit: 2 cores × MAX_BUFFER_ENTRIES=2
//
// Per-event cost ≈ measured_time / N.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_simulation_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("simulation/batch");

    // ── ProductionSimulator ──────────────────────────────────────────────────

    for &n in &[10usize, 100, 1000] {
        group.bench_with_input(BenchmarkId::new("run_until_idle/prod", n), &n, |b, &n| {
            b.iter_batched(
                || prod_sim_with_writes(n),
                |mut sim| black_box(sim.run_until_idle()),
                BatchSize::SmallInput,
            )
        });
    }

    // ── VerificationSimulator — max 4 events ─────────────────────────────────

    group.bench_function("run_until_idle/verif/4", |b| {
        b.iter_batched(
            || verif_sim_with_writes(4),
            |mut sim| black_box(sim.run_until_idle()),
            BatchSize::SmallInput,
        )
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 4: Reset
//
// Measures `simulator.reset()` — delegates to `memory.reset()` which clears:
//   - Main memory (DashMap clear / [u64; 4] zero-fill)
//   - Store buffers (VecDeque clear / [Option; 2] overwrite)
//   - Clock queue (BinaryHeap clear / fixed-array clear)
//
// Two pre-conditions:
//   empty     — freshly built, no events ever processed (minimal clear work)
//   after_run — write events committed, then reset (forces actual wipe)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_simulation_reset(c: &mut Criterion) {
    let mut group = c.benchmark_group("simulation/reset");

    // ── ProductionSimulator ──────────────────────────────────────────────────

    group.bench_function("reset/prod/empty", |b| {
        b.iter_batched(
            || {
                ProductionSimulatorBuilder::new()
                    .num_cores(4)
                    .buffer_size(2)
                    .build()
            },
            |mut sim| black_box(sim.reset()),
            BatchSize::SmallInput,
        )
    });

    // After committing 8 write events across 4 cores, reset must clear main memory
    group.bench_function("reset/prod/after_run", |b| {
        b.iter_batched(
            || {
                let mut sim = prod_sim_with_writes(8);
                sim.run_until_idle();
                sim
            },
            |mut sim| black_box(sim.reset()),
            BatchSize::SmallInput,
        )
    });

    // ── VerificationSimulator ────────────────────────────────────────────────

    group.bench_function("reset/verif/empty", |b| {
        b.iter_batched(
            || VerificationSimulatorBuilder::build(),
            |mut sim| black_box(sim.reset()),
            BatchSize::SmallInput,
        )
    });

    group.bench_function("reset/verif/after_run", |b| {
        b.iter_batched(
            || {
                let mut sim = verif_sim_with_writes(4);
                sim.run_until_idle();
                sim
            },
            |mut sim| black_box(sim.reset()),
            BatchSize::SmallInput,
        )
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 5: Dispatcher Isolation
//
// `EventDispatcher` is a stateless ZST (`pub struct EventDispatcher;`).
// These benchmarks bypass the `Simulator` wrapper and call the dispatcher methods
// directly on `&mut SimulatedMemory<_,_>` obtained via `sim.memory_mut()`.
//
// Goals:
//   - Prove that `EventDispatcher::new()` has near-zero cost (ZST construction)
//   - Show that `sim.step()` ≡ `dispatcher.process_event(sim.memory_mut())`
//     (no hidden overhead in the Simulator wrapper)
//   - Measure `process_all` loop overhead at small batch sizes
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_simulation_dispatcher(c: &mut Criterion) {
    let mut group = c.benchmark_group("simulation/dispatcher");

    // ZST construction — may be optimised away by compiler; black_box prevents it
    group.bench_function("new", |b| b.iter(|| black_box(EventDispatcher::new())));

    // Direct process_event on production memory (bypasses Simulator::step wrapper)
    let dispatcher = EventDispatcher::new();
    group.bench_function("process_event/prod/write", |b| {
        b.iter_batched(
            || prod_sim_with_writes(1),
            |mut sim| black_box(dispatcher.process_event(sim.memory_mut())),
            BatchSize::SmallInput,
        )
    });

    // Direct process_event with passthrough payload — zero-cost routing proof
    group.bench_function("process_event/prod/passthrough", |b| {
        b.iter_batched(
            || prod_sim_with_passthrough(),
            |mut sim| black_box(dispatcher.process_event(sim.memory_mut())),
            BatchSize::SmallInput,
        )
    });

    // process_all drain loop overhead with 10 events (shows loop amortisation)
    group.bench_function("process_all/prod/10", |b| {
        b.iter_batched(
            || prod_sim_with_writes(10),
            |mut sim| black_box(dispatcher.process_all(sim.memory_mut())),
            BatchSize::SmallInput,
        )
    });

    // Direct process_event on verification backend (no heap lock)
    group.bench_function("process_event/verif/write", |b| {
        b.iter_batched(
            || verif_sim_with_writes(1),
            |mut sim| black_box(dispatcher.process_event(sim.memory_mut())),
            BatchSize::SmallInput,
        )
    });

    // Direct process_event with verification passthrough
    group.bench_function("process_event/verif/passthrough", |b| {
        b.iter_batched(
            || verif_sim_with_passthrough(),
            |mut sim| black_box(dispatcher.process_event(sim.memory_mut())),
            BatchSize::SmallInput,
        )
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Criterion entry point
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

criterion_group!(
    name = simulation_benches;
    config = Criterion::default()
        .warm_up_time(std::time::Duration::from_millis(500))
        .measurement_time(std::time::Duration::from_secs(3));
    targets =
        bench_simulation_init,
        bench_simulation_step,
        bench_simulation_batch,
        bench_simulation_reset,
        bench_simulation_dispatcher
);

criterion_main!(simulation_benches);
