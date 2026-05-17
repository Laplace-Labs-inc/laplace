//! Resource Domain Benchmarks
//!
//! Measures performance characteristics of the `domain::resource` module:
//! - `resource/init`: Tracker construction, ID creation, quota limit lookups, usage snapshots
//! - `resource/tracking`: Hot-path request/release cycle, deadlock-check, score reads,
//!   tier-limit enforcement (`is_within_free_tier`)
//! - `resource/contention`: Multi-waiter block latency, release-with-wake latency,
//!   deadlock-check and contention-score overhead as waiting queue depth grows
//! - `resource/ffi_serialize`: JSON encode/decode of a `UsageSnapshot` wrapper and
//!   8-byte-aligned `Vec<u64>` mock-FFI buffer pack/unpack
//!
//! **Zero-Implementation Rule**: This file calls only the existing `domain::resource` API.
//! No new concurrency primitives, cgroup hooks, or OS-level allocations are introduced.
//!
//! **`DefaultTracker` = `DetailedTracker`** when `feature = "twin"` is active, providing
//! full wait-for-graph tracking, cycle detection, and Ki-DPOR heuristic metrics.

use criterion::{black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use laplace_core::domain::resource::{
    DefaultTracker, ResourceId, ResourceTracker, ResourceType, ResourceUsage, ThreadId,
};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// FFI alignment constant (matches laplace-interfaces FFI_BUFFER_ALIGN = 8)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

const FFI_BUFFER_ALIGN: usize = 8;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Local serializable wrapper for ResourceUsage
//
// ResourceUsage derives only Debug + Clone, not Serialize / Deserialize.
// We extract its public fields into this local mirror (same pattern as
// MemSnapshot in memory_bench.rs) for FFI serialization benchmarks.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(serde::Serialize, serde::Deserialize)]
struct UsageSnapshot {
    tenant_id: String,
    cpu_used_us: u64,
    memory_used_bytes: u64,
    network_used_bytes: u64,
    concurrent_requests: u32,
    storage_used_bytes: u64,
}

impl From<&ResourceUsage> for UsageSnapshot {
    fn from(u: &ResourceUsage) -> Self {
        Self {
            tenant_id: u.tenant_id.clone(),
            cpu_used_us: u.cpu_used_us,
            memory_used_bytes: u.memory_used_bytes,
            network_used_bytes: u.network_used_bytes,
            concurrent_requests: u.concurrent_requests,
            storage_used_bytes: u.storage_used_bytes,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Fixtures
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// All ResourceType variants for parameterised limit benchmarks.
const ALL_RESOURCE_TYPES: [ResourceType; 5] = [
    ResourceType::CpuMicroseconds,
    ResourceType::MemoryBytes,
    ResourceType::NetworkBytes,
    ResourceType::ConcurrentRequests,
    ResourceType::StorageBytes,
];

/// Tracker sizes benchmarked during initialisation.
const TRACKER_SIZES: [(usize, usize); 3] = [(1, 1), (4, 4), (8, 8)];

/// Waiter counts for contention scaling benchmarks.
/// Limited to 6 so setup tracker fits within MAX_THREADS = 8
/// (holder thread 0 + up to 6 waiters = 7 threads total).
const WAITER_COUNTS: [usize; 3] = [1, 3, 6];

/// Build a `ResourceUsage` snapshot with non-zero usage for all fields.
fn make_populated_usage() -> ResourceUsage {
    ResourceUsage {
        tenant_id: "bench-tenant-001".to_string(),
        cpu_used_us: 75_000,
        memory_used_bytes: 24 * 1024 * 1024,
        network_used_bytes: 8 * 1024 * 1024,
        concurrent_requests: 4,
        storage_used_bytes: 50 * 1024 * 1024,
    }
}

/// Build a tracker where thread 0 holds resource 0 and `n` additional
/// threads are blocked waiting for it.  Returned tracker is ready for
/// a read-only measurement of `has_deadlock()` or `contention_score()`.
fn tracker_with_n_waiters(n: usize) -> DefaultTracker {
    let num_threads = n + 1;
    let mut t = DefaultTracker::new(num_threads, 1);
    t.request(ThreadId(0), ResourceId(0)).unwrap(); // Thread 0 acquires
    for i in 1..=n {
        t.request(ThreadId(i), ResourceId(0)).unwrap(); // Threads 1..n block
    }
    t
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 1: Initialization — Tracker construction, ID creation, quota limits
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_resource_init(c: &mut Criterion) {
    let mut group = c.benchmark_group("resource/init");

    // Lightweight newtype wrappers (const fn → effectively zero cost)
    group.bench_function("thread_id", |b| {
        b.iter(|| black_box(ThreadId::new(black_box(3))))
    });

    group.bench_function("resource_id", |b| {
        b.iter(|| black_box(ResourceId::new(black_box(2))))
    });

    // ResourceUsage construction (allocates a String for tenant_id)
    group.bench_function("usage_new", |b| {
        b.iter(|| black_box(ResourceUsage::new(black_box("bench-tenant-001"))))
    });

    group.bench_function("usage_default", |b| {
        b.iter(|| black_box(ResourceUsage::default()))
    });

    // DefaultTracker::new — allocates fixed arrays + VecDeque instances.
    // Size (num_threads, num_resources) drives the initialisation cost.
    for (nt, nr) in TRACKER_SIZES {
        group.bench_with_input(
            BenchmarkId::new("tracker", format!("{}x{}", nt, nr)),
            &(nt, nr),
            |b, &(nt, nr)| b.iter(|| black_box(DefaultTracker::new(nt, nr))),
        );
    }

    // Per-tier quota limit lookups (pure match → ~1 ns)
    for rt in ALL_RESOURCE_TYPES {
        group.bench_with_input(
            BenchmarkId::new("limit_free", format!("{}", rt)),
            &rt,
            |b, &rt| b.iter(|| black_box(rt.default_limit_free())),
        );
        group.bench_with_input(
            BenchmarkId::new("limit_pro", format!("{}", rt)),
            &rt,
            |b, &rt| b.iter(|| black_box(rt.default_limit_pro())),
        );
        group.bench_with_input(
            BenchmarkId::new("limit_enterprise", format!("{}", rt)),
            &rt,
            |b, &rt| b.iter(|| black_box(rt.default_limit_enterprise())),
        );
    }

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 2: Tracking & Enforcement — Hot-path acquire/release, deadlock check,
//          score reads, and tier-limit enforcement
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_resource_tracking(c: &mut Criterion) {
    let mut group = c.benchmark_group("resource/tracking");

    // ── Acquire + release round-trip (cyclic, same thread, no contention) ────────
    //
    // Thread 0 acquires resource 0 (fast path: owner = None → set owner) then
    // releases it (no waiter → clear owner).  State is restored each iteration.
    {
        let mut tracker = DefaultTracker::new(2, 1);
        let t0 = ThreadId(0);
        let r0 = ResourceId(0);
        group.bench_function("acquire_release_roundtrip", |b| {
            b.iter(|| {
                tracker.request(black_box(t0), black_box(r0)).unwrap();
                tracker.release(black_box(t0), black_box(r0)).unwrap();
            })
        });
    }

    // ── Acquire only (fast path, resource free) ──────────────────────────────────
    //
    // Uses iter_batched so each iteration starts with a fresh, resource-free
    // tracker.  Measures the pure acquire overhead (owner = None → owner = t).
    group.bench_function("acquire_free", |b| {
        b.iter_batched(
            || DefaultTracker::new(1, 1),
            |mut t| black_box(t.request(ThreadId(0), ResourceId(0)).unwrap()),
            BatchSize::SmallInput,
        )
    });

    // ── Release only (no waiter, simple clear) ───────────────────────────────────
    //
    // Setup acquires the resource; bench measures only the release.
    group.bench_function("release_no_waiter", |b| {
        b.iter_batched(
            || {
                let mut t = DefaultTracker::new(1, 1);
                t.request(ThreadId(0), ResourceId(0)).unwrap();
                t
            },
            |mut t| black_box(t.release(ThreadId(0), ResourceId(0)).unwrap()),
            BatchSize::SmallInput,
        )
    });

    // ── Full cycle with on_finish ─────────────────────────────────────────────────
    //
    // Measures acquire → release → on_finish (thread lifecycle round-trip).
    group.bench_function("full_lifecycle", |b| {
        b.iter_batched(
            || DefaultTracker::new(1, 1),
            |mut t| {
                t.request(ThreadId(0), ResourceId(0)).unwrap();
                t.release(ThreadId(0), ResourceId(0)).unwrap();
                black_box(t.on_finish(ThreadId(0)).unwrap())
            },
            BatchSize::SmallInput,
        )
    });

    // ── has_deadlock on empty graph (O(1) — no edges) ────────────────────────────
    {
        let tracker = DefaultTracker::new(4, 4);
        group.bench_function("has_deadlock/empty", |b| {
            b.iter(|| black_box(tracker.has_deadlock()))
        });
    }

    // ── has_deadlock with one waiter (single wait-for edge, no cycle) ────────────
    {
        let t = tracker_with_n_waiters(1);
        group.bench_function("has_deadlock/one_waiter", |b| {
            b.iter(|| black_box(t.has_deadlock()))
        });
    }

    // ── deadlocked_threads on empty graph ────────────────────────────────────────
    {
        let tracker = DefaultTracker::new(4, 4);
        group.bench_function("deadlocked_threads/empty", |b| {
            b.iter(|| black_box(tracker.deadlocked_threads()))
        });
    }

    // ── contention_score (zero waiters) ─────────────────────────────────────────
    {
        let tracker = DefaultTracker::new(4, 4);
        group.bench_function("contention_score/zero", |b| {
            b.iter(|| black_box(tracker.contention_score()))
        });
    }

    // ── interleaving_score ───────────────────────────────────────────────────────
    {
        let tracker = DefaultTracker::new(4, 4);
        group.bench_function("interleaving_score", |b| {
            b.iter(|| black_box(tracker.interleaving_score()))
        });
    }

    // ── is_within_free_tier (zero usage — all predicates pass) ──────────────────
    {
        let usage = ResourceUsage::new("bench");
        group.bench_function("is_within_free_tier/within", |b| {
            b.iter(|| black_box(usage.is_within_free_tier()))
        });
    }

    // ── is_within_free_tier (at CPU limit — predicate fails on first field) ─────
    {
        let usage_over = ResourceUsage {
            tenant_id: "bench".to_string(),
            cpu_used_us: ResourceType::CpuMicroseconds.default_limit_free() + 1,
            memory_used_bytes: 0,
            network_used_bytes: 0,
            concurrent_requests: 0,
            storage_used_bytes: 0,
        };
        group.bench_function("is_within_free_tier/exceeded", |b| {
            b.iter(|| black_box(usage_over.is_within_free_tier()))
        });
    }

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 3: Concurrent Accounting — Multi-waiter overhead
//
// The DetailedTracker is single-writer (takes `&mut self`), but these benchmarks
// model the latency that the Axiom simulator observes when N threads contend on a
// single resource — the same path that a real concurrent workload would exercise
// through the scheduler.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_resource_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("resource/contention");

    // ── Block latency: cost of the Nth thread blocking on a held resource ────────
    //
    // Setup: thread 0 holds resource 0 and threads 1..(n-1) are already waiting.
    // Bench: thread n calls request() → appends to waiting_queues, updates
    //        wait_for_graph, increments context_switches, returns Blocked.
    for n in WAITER_COUNTS {
        group.bench_with_input(BenchmarkId::new("block_nth_waiter", n), &n, |b, &n| {
            b.iter_batched(
                || {
                    // Pre-fill: thread 0 holds, threads 1..n-1 already waiting
                    let num_threads = n + 1;
                    let mut t = DefaultTracker::new(num_threads, 1);
                    t.request(ThreadId(0), ResourceId(0)).unwrap();
                    for i in 1..n {
                        t.request(ThreadId(i), ResourceId(0)).unwrap();
                    }
                    t
                },
                |mut t| {
                    // Measure the Nth block call
                    black_box(t.request(ThreadId(n), ResourceId(0)).unwrap())
                },
                BatchSize::SmallInput,
            )
        });
    }

    // ── Release-with-wake latency: cost of waking the queue head ─────────────────
    //
    // Setup: thread 0 holds, threads 1..n are blocked.
    // Bench: thread 0 releases → pops VecDeque head, updates thread status,
    //        clears wait_for_graph edge, increments context_switches.
    for n in WAITER_COUNTS {
        group.bench_with_input(BenchmarkId::new("release_wake_waiter", n), &n, |b, &n| {
            b.iter_batched(
                || tracker_with_n_waiters(n),
                |mut t| black_box(t.release(ThreadId(0), ResourceId(0)).unwrap()),
                BatchSize::SmallInput,
            )
        });
    }

    // ── has_deadlock O(V+E) scan with N waiters ───────────────────────────────────
    //
    // The wait-for graph has N edges (each waiter → holder).  No cycle exists,
    // so DFS terminates exhausting all paths.  Measures the graph-scan overhead.
    for n in WAITER_COUNTS {
        let tracker = tracker_with_n_waiters(n);
        group.bench_function(&format!("has_deadlock/waiters_{}", n), |b| {
            b.iter(|| black_box(tracker.has_deadlock()))
        });
    }

    // ── contention_score sum across N waiting queues ─────────────────────────────
    //
    // Reads waiting_queues[0..num_resources].len() and sums them.
    // With N waiters all on resource 0, the sum equals N.
    for n in WAITER_COUNTS {
        let tracker = tracker_with_n_waiters(n);
        group.bench_function(&format!("contention_score/waiters_{}", n), |b| {
            b.iter(|| black_box(tracker.contention_score()))
        });
    }

    // ── interleaving_score after N blocks ────────────────────────────────────────
    //
    // Each block increments context_switches by 1.  Reading the counter
    // is a trivial load; we measure it after N blocks to match real workloads.
    for n in WAITER_COUNTS {
        let tracker = tracker_with_n_waiters(n);
        group.bench_function(&format!("interleaving_score/after_{}_blocks", n), |b| {
            b.iter(|| black_box(tracker.interleaving_score()))
        });
    }

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 4: FFI / Serialization Overhead
//
// Benchmarks the cost of exporting ResourceUsage metrics to the Axiom verification
// back-end or monitoring systems.  Uses a local `UsageSnapshot` wrapper (JSON
// wire format) packed into a `Vec<u64>` buffer that satisfies the 8-byte
// alignment contract of laplace-interfaces.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_resource_ffi_serialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("resource/ffi_serialize");

    // Pre-build fixtures outside the benchmark loops
    let zero_usage = ResourceUsage::new("bench-tenant-001");
    let full_usage = make_populated_usage();

    let zero_snap = UsageSnapshot::from(&zero_usage);
    let full_snap = UsageSnapshot::from(&full_usage);

    // ── JSON encode ──────────────────────────────────────────────────────────────

    group.bench_function("serialize_zero_usage", |b| {
        b.iter(|| black_box(serde_json::to_string(black_box(&zero_snap)).unwrap()))
    });

    group.bench_function("serialize_full_usage", |b| {
        b.iter(|| black_box(serde_json::to_string(black_box(&full_snap)).unwrap()))
    });

    // ── JSON decode ──────────────────────────────────────────────────────────────

    let json_full = serde_json::to_string(&full_snap).unwrap();
    group.bench_function("deserialize_full_usage", |b| {
        b.iter(|| {
            black_box(serde_json::from_str::<UsageSnapshot>(black_box(json_full.as_str())).unwrap())
        })
    });

    // ── Round-trip encode + decode ────────────────────────────────────────────────

    group.bench_function("roundtrip_full_usage", |b| {
        b.iter(|| {
            let encoded = serde_json::to_string(black_box(&full_snap)).unwrap();
            black_box(serde_json::from_str::<UsageSnapshot>(&encoded).unwrap())
        })
    });

    // ── FFI buffer pack: JSON → Vec<u64> (8-byte-aligned write direction) ────────
    //
    // Vec<u64> guarantees align(8) — satisfies FFI_BUFFER_ALIGN contract.
    // Models staging a resource-usage report for transmission to V8 / Axiom.
    group.bench_function("ffi_buffer_pack", |b| {
        b.iter(|| {
            let json = serde_json::to_string(black_box(&full_snap)).unwrap();
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

    // ── FFI buffer unpack: Vec<u64> → decoded UsageSnapshot ──────────────────────

    let pack_json = serde_json::to_string(&full_snap).unwrap();
    let pack_bytes = pack_json.as_bytes();
    let pack_word_count = pack_bytes.len().div_ceil(FFI_BUFFER_ALIGN);
    let mut pack_buf: Vec<u64> = vec![0u64; pack_word_count];
    let pack_byte_len = pack_bytes.len();
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
            black_box(serde_json::from_slice::<UsageSnapshot>(black_box(slice)).unwrap())
        })
    });

    // ── FFI buffer full round-trip: encode → pack → unpack → decode ──────────────

    group.bench_function("ffi_buffer_roundtrip", |b| {
        b.iter(|| {
            // Encode
            let json = serde_json::to_string(black_box(&full_snap)).unwrap();
            let bytes = json.as_bytes();
            let word_count = bytes.len().div_ceil(FFI_BUFFER_ALIGN);
            let mut buf: Vec<u64> = vec![0u64; word_count];
            let byte_len = bytes.len();
            // Pack
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    buf.as_mut_ptr() as *mut u8,
                    byte_len,
                );
            }
            // Unpack + decode
            let slice = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, byte_len) };
            black_box(serde_json::from_slice::<UsageSnapshot>(slice).unwrap())
        })
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Criterion entry point
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

criterion_group!(
    name = resource_benches;
    config = Criterion::default()
        .warm_up_time(std::time::Duration::from_millis(500))
        .measurement_time(std::time::Duration::from_secs(3));
    targets =
        bench_resource_init,
        bench_resource_tracking,
        bench_resource_contention,
        bench_resource_ffi_serialize
);

criterion_main!(resource_benches);
