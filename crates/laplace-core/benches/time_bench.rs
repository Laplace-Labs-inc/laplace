//! Time Domain Benchmarks
//!
//! Measures performance characteristics of the `domain::time` module:
//! - `time/init`       – Production (`BinaryHeap`) vs Verification (Fixed Array) backend initialization.
//! - `time/ops`        – Sub-nanosecond operations (Lamport clock, time read/write).
//! - `time/push_pop`   – Queue operation throughput. Exposes O(log N) heap vs O(N) array scaling.
//! - `time/ffi_serialize` – JSON encode/decode of Time metrics + 8-byte aligned FFI buffer.
//!
//! **Zero-Implementation Rule**: all calls use only the existing public API.

use criterion::{black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use laplace_core::domain::time::{
    ClockBackend, EventPayload, ProductionBackend, ScheduledEvent, VerificationBackend,
};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// FFI alignment constant (matches laplace-interfaces FFI_BUFFER_ALIGN = 8)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
const FFI_BUFFER_ALIGN: usize = 8;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Local serializable snapshot
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
#[derive(serde::Serialize, serde::Deserialize)]
struct TimeSnapshot {
    virtual_time_ns: u64,
    lamport_clock: u64,
    queue_len: usize,
    backend_type: String,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Setup helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn fill_prod_backend(n: u64) -> ProductionBackend {
    let mut backend = ProductionBackend::new();
    for i in 0..n {
        backend.push_event(ScheduledEvent::new(1000 + i, i, i, EventPayload::Test(i)));
    }
    backend
}

// Verification backend has capacity limit (usually 64 in non-Kani).
fn fill_verif_backend(n: u64) -> VerificationBackend {
    let mut backend = VerificationBackend::new();
    for i in 0..n {
        backend.push_event(ScheduledEvent::new(1000 + i, i, i, EventPayload::Test(i)));
    }
    backend
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 1: Initialization
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_time_init(c: &mut Criterion) {
    let mut group = c.benchmark_group("time/init");

    // Production backend (Heap allocation)
    group.bench_function("production_backend", |b| {
        b.iter(|| black_box(ProductionBackend::new()))
    });

    // Verification backend (Stack allocation / Fixed array)
    group.bench_function("verification_backend", |b| {
        b.iter(|| black_box(VerificationBackend::new()))
    });

    // ScheduledEvent creation overhead
    group.bench_function("scheduled_event_new", |b| {
        b.iter(|| {
            black_box(ScheduledEvent::new(
                black_box(100),
                black_box(1),
                black_box(1),
                black_box(EventPayload::Test(42)),
            ))
        })
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 2: Clock Operations (Lamport & Virtual Time)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_time_ops(c: &mut Criterion) {
    let mut group = c.benchmark_group("time/ops");
    let mut prod = ProductionBackend::new();
    prod.set_time(5000);

    group.bench_function("current_time", |b| {
        b.iter(|| black_box(prod.current_time()))
    });

    group.bench_function("current_lamport", |b| {
        b.iter(|| black_box(prod.current_lamport()))
    });

    group.bench_function("increment_lamport", |b| {
        b.iter(|| black_box(prod.increment_lamport()))
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 3: Push / Pop Throughput
//
// Highlights the architectural difference:
// Production uses BinaryHeap -> O(log N)
// Verification uses Fixed Array Option Slots -> O(N) scan
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_time_push_pop(c: &mut Criterion) {
    let mut group = c.benchmark_group("time/push_pop");

    // ── Push Operations ──────────────────────────────────────────────────────
    let prod_sizes = [10u64, 100, 1000];
    for &n in &prod_sizes {
        group.bench_with_input(BenchmarkId::new("push/prod", n), &n, |b, &n| {
            b.iter_batched(
                || fill_prod_backend(n),
                |mut backend| {
                    backend.push_event(ScheduledEvent::new(500, 0, 9999, EventPayload::Test(0)))
                },
                BatchSize::SmallInput,
            )
        });
    }

    let verif_sizes = [10u64, 50]; // Verification max is usually 64
    for &n in &verif_sizes {
        group.bench_with_input(BenchmarkId::new("push/verif", n), &n, |b, &n| {
            b.iter_batched(
                || fill_verif_backend(n),
                |mut backend| {
                    backend.push_event(ScheduledEvent::new(500, 0, 9999, EventPayload::Test(0)))
                },
                BatchSize::SmallInput,
            )
        });
    }

    // ── Pop Operations ───────────────────────────────────────────────────────
    for &n in &prod_sizes {
        group.bench_with_input(BenchmarkId::new("pop/prod", n), &n, |b, &n| {
            b.iter_batched(
                || fill_prod_backend(n),
                |mut backend| black_box(backend.pop_event()),
                BatchSize::SmallInput,
            )
        });
    }

    for &n in &verif_sizes {
        group.bench_with_input(BenchmarkId::new("pop/verif", n), &n, |b, &n| {
            b.iter_batched(
                || fill_verif_backend(n),
                |mut backend| black_box(backend.pop_event()),
                BatchSize::SmallInput,
            )
        });
    }

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 4: FFI / Serialization Overhead
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_time_ffi_serialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("time/ffi_serialize");

    let snap = TimeSnapshot {
        virtual_time_ns: 1_000_000,
        lamport_clock: 42,
        queue_len: 128,
        backend_type: "Production".to_string(),
    };

    group.bench_function("serialize", |b| {
        b.iter(|| black_box(serde_json::to_string(black_box(&snap)).unwrap()))
    });

    let json_snap = serde_json::to_string(&snap).unwrap();
    group.bench_function("deserialize", |b| {
        b.iter(|| {
            black_box(serde_json::from_str::<TimeSnapshot>(black_box(json_snap.as_str())).unwrap())
        })
    });

    group.bench_function("ffi_buffer_roundtrip", |b| {
        b.iter(|| {
            let json = serde_json::to_string(black_box(&snap)).unwrap();
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

            black_box(serde_json::from_slice::<TimeSnapshot>(slice).unwrap())
        })
    });

    group.finish();
}

criterion_group!(
    name = time_benches;
    config = Criterion::default()
        .warm_up_time(std::time::Duration::from_millis(500))
        .measurement_time(std::time::Duration::from_secs(3));
    targets =
        bench_time_init,
        bench_time_ops,
        bench_time_push_pop,
        bench_time_ffi_serialize
);

criterion_main!(time_benches);
