//! Scheduler Domain Benchmarks
//!
//! Measures performance characteristics of the `domain::scheduler` module:
//! - `scheduler/init`          – Backend and engine construction overhead.
//! - `scheduler/enqueue`       – Event register/unregister (enqueue/dequeue)
//!                               on `ProductionBackend` (`DashMap` + `parking_lot::RwLock`)
//!                               and `VerificationBackend` (`RefCell` + fixed array).
//! - `scheduler/state`         – Thread state get/set/count transitions
//!                               and predicate methods.
//! - `scheduler/priority`      – Runnable-event scan (O(N) "priority pop"),
//!                               `schedule_task` state-gate validation, idle detection.
//! - `scheduler/ffi_serialize` – JSON encode/decode of a `SchedulerSnapshot` wrapper
//!                               and 8-byte-aligned `Vec<u64>` mock-FFI buffer.
//!
//! **Zero-Implementation Rule**: all calls use only the existing `domain::scheduler` API.
//! No OS thread spawning or scheduling loop logic is introduced.

use criterion::{black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use laplace_core::domain::scheduler::{
    ProductionBackend, ProductionScheduler, SchedulerBackend, SchedulingStrategy, TaskId, ThreadId,
    ThreadState, VerificationBackend, VerificationScheduler,
};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// FFI alignment constant (matches laplace-interfaces FFI_BUFFER_ALIGN = 8)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

const FFI_BUFFER_ALIGN: usize = 8;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Local serializable snapshot
//
// `SchedulerEngine` and its backends do not derive `Serialize`.  We extract
// the metrics visible through the public API into this wrapper — the same
// pattern used in memory_bench (MemSnapshot) and resource_bench (UsageSnapshot).
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(serde::Serialize, serde::Deserialize)]
struct SchedulerSnapshot {
    num_threads: usize,
    runnable_count: usize,
    blocked_count: usize,
    completed_count: usize,
    runnable_events: usize,
    is_idle: bool,
    strategy: String,
}

fn snap_from_prod(engine: &ProductionScheduler) -> SchedulerSnapshot {
    let (r, b, c) = engine.thread_state_counts();
    SchedulerSnapshot {
        num_threads: engine.num_threads(),
        runnable_count: r,
        blocked_count: b,
        completed_count: c,
        runnable_events: engine.backend().count_runnable_events(),
        is_idle: engine.is_idle(),
        strategy: SchedulingStrategy::Production.to_string(),
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Constants
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Thread counts for parameterised construction benchmarks.
const THREAD_COUNTS: [usize; 3] = [4, 8, 16];

/// Event counts for enqueue and runnable-scan benchmarks.
/// Capped at 8 — the `VerificationBackend` MAX_EVENTS limit.
const EVENT_COUNTS: [usize; 3] = [1, 4, 8];

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 1: Initialization
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_scheduler_init(c: &mut Criterion) {
    let mut group = c.benchmark_group("scheduler/init");

    // Newtype wrappers (const fn — effectively zero cost)
    group.bench_function("thread_id", |b| {
        b.iter(|| black_box(ThreadId::new(black_box(3))))
    });
    group.bench_function("task_id", |b| {
        b.iter(|| black_box(TaskId::new(black_box(0))))
    });

    // ProductionBackend::new — allocates DashMap and inserts N Runnable entries.
    for &n in &THREAD_COUNTS {
        group.bench_with_input(BenchmarkId::new("production_backend", n), &n, |b, &n| {
            b.iter(|| black_box(ProductionBackend::new(n)))
        });
    }

    // VerificationBackend::new — zeroes a fixed [ThreadState; 4] on the stack.
    group.bench_function("verification_backend", |b| {
        b.iter(|| black_box(VerificationBackend::new(4)))
    });

    // ProductionScheduler::new — engine wrapping ProductionBackend.
    for &n in &[4usize, 8] {
        group.bench_with_input(BenchmarkId::new("production_scheduler", n), &n, |b, &n| {
            b.iter(|| black_box(ProductionScheduler::new(n, SchedulingStrategy::Production)))
        });
    }

    // VerificationScheduler::new — engine wrapping VerificationBackend (≤ 4 threads).
    group.bench_function("verification_scheduler", |b| {
        b.iter(|| {
            black_box(VerificationScheduler::new(
                4,
                SchedulingStrategy::Verification,
            ))
        })
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 2: Enqueue / Dequeue Throughput
//
// "Enqueue" = `register_event`   (write-locks `RwLock<HashMap>` for Production;
//                                  `borrow_mut` on `RefCell` for Verification).
// "Dequeue" = `unregister_event` (same lock paths).
//
// Measuring both backends side-by-side reveals the raw lock overhead that
// multiple concurrent threads would incur in production deployments.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_scheduler_enqueue(c: &mut Criterion) {
    let mut group = c.benchmark_group("scheduler/enqueue");

    // ── ProductionBackend (parking_lot::RwLock<HashMap>) ─────────────────────

    // register + unregister round-trip (cyclic — state restores after each iter)
    {
        let backend = ProductionBackend::new(4);
        group.bench_function("register_roundtrip/prod", |b| {
            b.iter(|| {
                backend
                    .register_event(black_box(42u64), black_box(ThreadId::new(0)))
                    .unwrap();
                backend.unregister_event(black_box(42u64));
            })
        });
    }

    // register_event only — exclusive write-lock acquire + HashMap insert
    group.bench_function("register_only/prod", |b| {
        b.iter_batched(
            || ProductionBackend::new(4),
            |backend| black_box(backend.register_event(42u64, ThreadId::new(0)).unwrap()),
            BatchSize::SmallInput,
        )
    });

    // unregister_event only — pre-register in setup, measure single HashMap remove
    group.bench_function("unregister_only/prod", |b| {
        b.iter_batched(
            || {
                let backend = ProductionBackend::new(4);
                backend.register_event(42u64, ThreadId::new(0)).unwrap();
                backend
            },
            |backend| black_box(backend.unregister_event(42u64)),
            BatchSize::SmallInput,
        )
    });

    // get_event_owner — shared read-lock acquire + HashMap lookup
    {
        let backend = ProductionBackend::new(4);
        backend.register_event(99u64, ThreadId::new(1)).unwrap();
        group.bench_function("get_owner/prod", |b| {
            b.iter(|| black_box(backend.get_event_owner(black_box(99u64))))
        });
    }

    // count_runnable_events with N pre-registered events
    // (read lock + O(N) DashMap + is_runnable per event)
    for &n in &EVENT_COUNTS {
        let backend = ProductionBackend::new(8);
        for i in 0..n {
            backend
                .register_event(i as u64, ThreadId::new(i % 8))
                .unwrap();
        }
        group.bench_function(&format!("count_runnable_events/prod/{n}"), |b| {
            b.iter(|| black_box(backend.count_runnable_events()))
        });
    }

    // schedule_task on ProductionScheduler:
    //   get_state (DashMap read) + now_ns + generate_event_id + register_event (write lock)
    group.bench_function("schedule_task/prod", |b| {
        b.iter_batched(
            || ProductionScheduler::new(4, SchedulingStrategy::Production),
            |mut engine| black_box(engine.schedule_task(ThreadId::new(0), 100_000_000).unwrap()),
            BatchSize::SmallInput,
        )
    });

    // clear_events — exclusive write lock + HashMap::clear
    group.bench_function("clear_events/prod", |b| {
        b.iter_batched(
            || {
                let backend = ProductionBackend::new(4);
                for i in 0..8u64 {
                    backend.register_event(i, ThreadId::new(0)).unwrap();
                }
                backend
            },
            |backend| black_box(backend.clear_events()),
            BatchSize::SmallInput,
        )
    });

    // ── VerificationBackend (RefCell<fixed-array>) ───────────────────────────

    // register + unregister round-trip (cyclic, RefCell — no external lock)
    {
        let backend = VerificationBackend::new(4);
        group.bench_function("register_roundtrip/verif", |b| {
            b.iter(|| {
                backend
                    .register_event(black_box(42u64), black_box(ThreadId::new(0)))
                    .unwrap();
                backend.unregister_event(black_box(42u64));
            })
        });
    }

    // schedule_task on VerificationScheduler (stack-allocated path, no heap lock)
    group.bench_function("schedule_task/verif", |b| {
        b.iter_batched(
            || VerificationScheduler::new(4, SchedulingStrategy::Verification),
            |mut engine| black_box(engine.schedule_task(ThreadId::new(0), 100_000_000).unwrap()),
            BatchSize::SmallInput,
        )
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 3: Task State Transitions
//
// Each mutation on `ProductionBackend` passes through a DashMap sharded write
// lock; on `VerificationBackend` it uses a `RefCell::borrow_mut`.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_scheduler_state(c: &mut Criterion) {
    let mut group = c.benchmark_group("scheduler/state");

    // ── get_state ────────────────────────────────────────────────────────────

    {
        let prod = ProductionBackend::new(8);
        group.bench_function("get_state/prod", |b| {
            b.iter(|| black_box(prod.get_state(black_box(ThreadId::new(0))).unwrap()))
        });
    }
    {
        let verif = VerificationBackend::new(4);
        group.bench_function("get_state/verif", |b| {
            b.iter(|| black_box(verif.get_state(black_box(ThreadId::new(0))).unwrap()))
        });
    }

    // ── set_state RUNNABLE ↔ BLOCKED round-trip (hot transition path) ────────

    {
        let prod = ProductionBackend::new(8);
        group.bench_function("set_state_roundtrip/prod", |b| {
            b.iter(|| {
                prod.set_state(black_box(ThreadId::new(0)), black_box(ThreadState::Blocked))
                    .unwrap();
                prod.set_state(
                    black_box(ThreadId::new(0)),
                    black_box(ThreadState::Runnable),
                )
                .unwrap();
            })
        });
    }
    {
        let verif = VerificationBackend::new(4);
        group.bench_function("set_state_roundtrip/verif", |b| {
            b.iter(|| {
                verif
                    .set_state(black_box(ThreadId::new(0)), black_box(ThreadState::Blocked))
                    .unwrap();
                verif
                    .set_state(
                        black_box(ThreadId::new(0)),
                        black_box(ThreadState::Runnable),
                    )
                    .unwrap();
            })
        });
    }

    // ── RUNNABLE ↔ BLOCKED round-trip via SchedulerEngine ────────────────────
    // Engine adds bounds-check before delegating to backend.
    {
        let mut engine = ProductionScheduler::new(8, SchedulingStrategy::Production);
        group.bench_function("engine_set_state_roundtrip/prod", |b| {
            b.iter(|| {
                engine
                    .set_thread_state(black_box(ThreadId::new(0)), ThreadState::Blocked)
                    .unwrap();
                engine
                    .set_thread_state(black_box(ThreadId::new(0)), ThreadState::Runnable)
                    .unwrap();
            })
        });
    }

    // ── state_counts — O(N) full scan ────────────────────────────────────────

    {
        let prod = ProductionBackend::new(8);
        prod.set_state(ThreadId::new(0), ThreadState::Blocked)
            .unwrap();
        prod.set_state(ThreadId::new(1), ThreadState::Completed)
            .unwrap();
        group.bench_function("state_counts/prod", |b| {
            b.iter(|| black_box(prod.state_counts()))
        });
    }
    {
        let verif = VerificationBackend::new(4);
        verif
            .set_state(ThreadId::new(0), ThreadState::Blocked)
            .unwrap();
        verif
            .set_state(ThreadId::new(1), ThreadState::Completed)
            .unwrap();
        group.bench_function("state_counts/verif", |b| {
            b.iter(|| black_box(verif.state_counts()))
        });
    }
    {
        let mut engine = ProductionScheduler::new(8, SchedulingStrategy::Production);
        engine
            .set_thread_state(ThreadId::new(0), ThreadState::Blocked)
            .unwrap();
        group.bench_function("thread_state_counts/engine", |b| {
            b.iter(|| black_box(engine.thread_state_counts()))
        });
    }

    // ── is_runnable convenience check ────────────────────────────────────────

    {
        let prod = ProductionBackend::new(8);
        group.bench_function("is_runnable/prod", |b| {
            b.iter(|| black_box(prod.is_runnable(black_box(ThreadId::new(0)))))
        });
    }
    {
        let verif = VerificationBackend::new(4);
        group.bench_function("is_runnable/verif", |b| {
            b.iter(|| black_box(verif.is_runnable(black_box(ThreadId::new(0)))))
        });
    }

    // ── ThreadState enum predicate methods (pure match, ~sub-ns cost) ────────
    group.bench_function("thread_state/is_runnable", |b| {
        b.iter(|| black_box(black_box(ThreadState::Runnable).is_runnable()))
    });
    group.bench_function("thread_state/is_blocked", |b| {
        b.iter(|| black_box(black_box(ThreadState::Blocked).is_blocked()))
    });
    group.bench_function("thread_state/is_completed", |b| {
        b.iter(|| black_box(black_box(ThreadState::Completed).is_completed()))
    });

    // ── SchedulingStrategy predicates ────────────────────────────────────────
    group.bench_function("strategy/is_production", |b| {
        b.iter(|| black_box(black_box(SchedulingStrategy::Production).is_production()))
    });
    group.bench_function("strategy/is_verification", |b| {
        b.iter(|| black_box(black_box(SchedulingStrategy::Verification).is_verification()))
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 4: Priority Scheduling
//
// The scheduler enforces implicit "priority" via thread-state validation:
// only RUNNABLE threads can have tasks scheduled, and `count_runnable_events`
// is the O(N) "priority pop" scan that selects eligible events for execution.
//
// Benchmarks expose:
//  - How `schedule_task` validation cost scales with N threads (DashMap shards).
//  - How the runnable-event scan degrades as more threads become blocked.
//  - Idle detection and full scheduler reset overhead.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_scheduler_priority(c: &mut Criterion) {
    let mut group = c.benchmark_group("scheduler/priority");

    // ── schedule_task scaling with N threads ─────────────────────────────────
    // schedule_task path: get_state (DashMap shard read) → time check
    //                   → generate_event_id → register_event (RwLock write)
    for &n in &[4usize, 8] {
        group.bench_with_input(BenchmarkId::new("schedule_task/threads", n), &n, |b, &n| {
            b.iter_batched(
                || ProductionScheduler::new(n, SchedulingStrategy::Production),
                |mut engine| black_box(engine.schedule_task(ThreadId::new(0), 1_000_000).unwrap()),
                BatchSize::SmallInput,
            )
        });
    }

    // ── count_runnable_events — O(N) scan (all threads RUNNABLE) ─────────────
    // Measures per-event cost when every event is selectable.
    for &n in &EVENT_COUNTS {
        let backend = ProductionBackend::new(8);
        for i in 0..n {
            backend
                .register_event(i as u64, ThreadId::new(i % 8))
                .unwrap();
        }
        group.bench_function(&format!("count_runnable_events/all_runnable/{n}"), |b| {
            b.iter(|| black_box(backend.count_runnable_events()))
        });
    }

    // ── count_runnable_events — half threads blocked ──────────────────────────
    // 4 of 8 threads blocked → their events are non-runnable.
    // Models the realistic mixed-state workload.
    {
        let backend = ProductionBackend::new(8);
        for i in 0..8u64 {
            backend
                .register_event(i, ThreadId::new(i as usize % 8))
                .unwrap();
        }
        for i in [0usize, 2, 4, 6] {
            backend
                .set_state(ThreadId::new(i), ThreadState::Blocked)
                .unwrap();
        }
        group.bench_function("count_runnable_events/half_blocked", |b| {
            b.iter(|| black_box(backend.count_runnable_events()))
        });
    }

    // ── is_idle — checks count_runnable_events == 0 ──────────────────────────
    {
        let engine = ProductionScheduler::new(4, SchedulingStrategy::Production);
        group.bench_function("is_idle/empty", |b| b.iter(|| black_box(engine.is_idle())));
    }

    // ── reset — restore all threads to RUNNABLE + clear event map ────────────
    group.bench_function("reset/prod", |b| {
        b.iter_batched(
            || {
                let mut engine = ProductionScheduler::new(8, SchedulingStrategy::Production);
                engine
                    .set_thread_state(ThreadId::new(0), ThreadState::Blocked)
                    .unwrap();
                engine
                    .set_thread_state(ThreadId::new(1), ThreadState::Completed)
                    .unwrap();
                engine
            },
            |mut engine| black_box(engine.reset()),
            BatchSize::SmallInput,
        )
    });

    group.bench_function("reset/verif", |b| {
        b.iter_batched(
            || {
                let mut engine = VerificationScheduler::new(4, SchedulingStrategy::Verification);
                engine
                    .set_thread_state(ThreadId::new(0), ThreadState::Blocked)
                    .unwrap();
                engine
            },
            |mut engine| black_box(engine.reset()),
            BatchSize::SmallInput,
        )
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 5: FFI / Serialization Overhead
//
// Models the cost of exporting scheduler metrics to the Axiom verification
// back-end or monitoring systems.  `SchedulerSnapshot` is a local wrapper
// populated from the existing public API; bytes are then packed into a
// `Vec<u64>` buffer satisfying the 8-byte alignment contract.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_scheduler_ffi_serialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("scheduler/ffi_serialize");

    // Build snapshots once outside the timing loop
    let idle_engine = ProductionScheduler::new(4, SchedulingStrategy::Production);
    let idle_snap = snap_from_prod(&idle_engine);

    let mut active_engine = ProductionScheduler::new(8, SchedulingStrategy::Production);
    active_engine
        .set_thread_state(ThreadId::new(0), ThreadState::Blocked)
        .unwrap();
    active_engine
        .set_thread_state(ThreadId::new(1), ThreadState::Completed)
        .unwrap();
    let active_snap = snap_from_prod(&active_engine);

    // ── JSON encode ──────────────────────────────────────────────────────────

    group.bench_function("serialize_idle", |b| {
        b.iter(|| black_box(serde_json::to_string(black_box(&idle_snap)).unwrap()))
    });
    group.bench_function("serialize_active", |b| {
        b.iter(|| black_box(serde_json::to_string(black_box(&active_snap)).unwrap()))
    });

    // ── JSON decode ──────────────────────────────────────────────────────────

    let json_active = serde_json::to_string(&active_snap).unwrap();
    group.bench_function("deserialize_active", |b| {
        b.iter(|| {
            black_box(
                serde_json::from_str::<SchedulerSnapshot>(black_box(json_active.as_str())).unwrap(),
            )
        })
    });

    // ── Round-trip ───────────────────────────────────────────────────────────

    group.bench_function("roundtrip_active", |b| {
        b.iter(|| {
            let encoded = serde_json::to_string(black_box(&active_snap)).unwrap();
            black_box(serde_json::from_str::<SchedulerSnapshot>(&encoded).unwrap())
        })
    });

    // ── FFI buffer pack: JSON → Vec<u64> (8-byte-aligned write direction) ────
    //
    // `Vec<u64>` guarantees align(8) — satisfies FFI_BUFFER_ALIGN contract.
    // Models staging a scheduler status report for transmission to V8 / Axiom.
    group.bench_function("ffi_buffer_pack", |b| {
        b.iter(|| {
            let json = serde_json::to_string(black_box(&active_snap)).unwrap();
            let bytes = json.as_bytes();
            let word_count = bytes.len().div_ceil(FFI_BUFFER_ALIGN);
            let mut buf: Vec<u64> = vec![0u64; word_count];
            // Safety: buf is word_count * 8 bytes ≥ bytes.len()
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    buf.as_mut_ptr() as *mut u8,
                    bytes.len(),
                );
            }
            black_box(buf)
        })
    });

    // ── FFI buffer unpack: Vec<u64> → decoded SchedulerSnapshot ─────────────

    let pack_json = serde_json::to_string(&active_snap).unwrap();
    let pack_bytes = pack_json.as_bytes();
    let pack_word_count = pack_bytes.len().div_ceil(FFI_BUFFER_ALIGN);
    let mut pack_buf: Vec<u64> = vec![0u64; pack_word_count];
    let pack_byte_len = pack_bytes.len();
    // Safety: pack_buf is pack_word_count * 8 bytes ≥ pack_byte_len
    unsafe {
        std::ptr::copy_nonoverlapping(
            pack_bytes.as_ptr(),
            pack_buf.as_mut_ptr() as *mut u8,
            pack_byte_len,
        );
    }

    group.bench_function("ffi_buffer_unpack", |b| {
        b.iter(|| {
            // Safety: pack_buf contains valid UTF-8 JSON written above
            let slice = unsafe {
                std::slice::from_raw_parts(pack_buf.as_ptr() as *const u8, pack_byte_len)
            };
            black_box(serde_json::from_slice::<SchedulerSnapshot>(black_box(slice)).unwrap())
        })
    });

    // ── Full FFI round-trip ──────────────────────────────────────────────────

    group.bench_function("ffi_buffer_roundtrip", |b| {
        b.iter(|| {
            let json = serde_json::to_string(black_box(&active_snap)).unwrap();
            let bytes = json.as_bytes();
            let word_count = bytes.len().div_ceil(FFI_BUFFER_ALIGN);
            let mut buf: Vec<u64> = vec![0u64; word_count];
            let byte_len = bytes.len();
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    buf.as_mut_ptr() as *mut u8,
                    byte_len,
                );
            }
            let slice = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, byte_len) };
            black_box(serde_json::from_slice::<SchedulerSnapshot>(slice).unwrap())
        })
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Criterion entry point
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

criterion_group!(
    name = scheduler_benches;
    config = Criterion::default()
        .warm_up_time(std::time::Duration::from_millis(500))
        .measurement_time(std::time::Duration::from_secs(3));
    targets =
        bench_scheduler_init,
        bench_scheduler_enqueue,
        bench_scheduler_state,
        bench_scheduler_priority,
        bench_scheduler_ffi_serialize
);

criterion_main!(scheduler_benches);
