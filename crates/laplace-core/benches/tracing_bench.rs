// SPDX-License-Identifier: Apache-2.0
//! Tracing Domain Benchmarks
//!
//! Measures performance characteristics of the `domain::tracing` module:
//! - `tracing/init`       – Production (`Vec` backed) vs Verification (Array backed) backend initialization.
//! - `tracing/logging`    – Event logging throughput (`log_read`, `log_write`).
//! - `tracing/causality`  – Causality verification and `CausalityGraph` construction.
//! - `tracing/ffi_serialize` – JSON encode/decode of events and 8-byte aligned FFI buffer pack/unpack.
//!
//! **Zero-Implementation Rule**: all calls use only the existing public API.

use criterion::{black_box, criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use laplace_core::domain::memory::Address;
use laplace_core::domain::tracing::causality::CausalityGraph;
use laplace_core::domain::tracing::traits::TracerBackend;
use laplace_core::domain::tracing::types::{
    EventMetadata, LamportTimestamp, MemoryOperation, SimulationEvent, ThreadId,
};
use laplace_core::domain::tracing::{
    ProductionBackend, ProductionTracer, TraceEngine, TraceEngineConfig, VerificationBackend,
    VerificationTracer,
};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// FFI alignment constant (matches laplace-interfaces FFI_BUFFER_ALIGN = 8)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
const FFI_BUFFER_ALIGN: usize = 8;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Setup helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn fill_prod_backend(n: usize) -> ProductionBackend {
    let mut backend = ProductionBackend::new(n.max(100));
    let tid = ThreadId::new(0);
    for i in 0..n {
        let meta = EventMetadata::new(LamportTimestamp(i as u64), tid, i as u64);
        let event = SimulationEvent::Memory {
            meta,
            operation: MemoryOperation::Read {
                addr: Address::new(i),
                value: i as u64,
                cache_hit: true,
            },
        };
        backend.append_event(event).unwrap();
    }
    backend
}

fn fill_verif_backend(n: usize) -> VerificationBackend {
    let mut backend = VerificationBackend::new();
    let tid = ThreadId::new(0);
    for i in 0..n {
        let meta = EventMetadata::new(LamportTimestamp(i as u64), tid, i as u64);
        let event = SimulationEvent::Memory {
            meta,
            operation: MemoryOperation::Read {
                addr: Address::new(i),
                value: i as u64,
                cache_hit: true,
            },
        };
        backend.append_event(event).unwrap();
    }
    backend
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 1: Initialization
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_tracing_init(c: &mut Criterion) {
    let mut group = c.benchmark_group("tracing/init");

    let capacities = [1000, 10_000, 100_000];
    for &cap in &capacities {
        group.bench_with_input(
            BenchmarkId::new("production_backend", cap),
            &cap,
            |b, &cap| b.iter(|| black_box(ProductionBackend::new(cap))),
        );
    }

    group.bench_function("verification_backend", |b| {
        b.iter(|| black_box(VerificationBackend::new()))
    });

    group.bench_function("production_tracer", |b| {
        b.iter_batched(
            || ProductionBackend::new(1000),
            |backend| black_box(TraceEngine::new(backend, TraceEngineConfig::default())),
            BatchSize::SmallInput,
        )
    });

    group.bench_function("verification_tracer", |b| {
        b.iter_batched(
            || VerificationBackend::new(),
            |backend| black_box(TraceEngine::new(backend, TraceEngineConfig::default())),
            BatchSize::SmallInput,
        )
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 2: Logging Operations
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_tracing_logging(c: &mut Criterion) {
    let mut group = c.benchmark_group("tracing/logging");

    group.bench_function("log_read/prod", |b| {
        b.iter_batched(
            || {
                let backend = ProductionBackend::new(1000);
                ProductionTracer::new(backend, TraceEngineConfig::default())
            },
            |mut tracer| {
                black_box(
                    tracer
                        .log_read(ThreadId::new(0), Address::new(0x1000), 42)
                        .unwrap(),
                )
            },
            BatchSize::SmallInput,
        )
    });

    group.bench_function("log_write/prod", |b| {
        b.iter_batched(
            || {
                let backend = ProductionBackend::new(1000);
                ProductionTracer::new(backend, TraceEngineConfig::default())
            },
            |mut tracer| {
                black_box(
                    tracer
                        .log_write(ThreadId::new(0), Address::new(0x1000), 42, false)
                        .unwrap(),
                )
            },
            BatchSize::SmallInput,
        )
    });

    group.bench_function("log_read/verif", |b| {
        b.iter_batched(
            || {
                let backend = VerificationBackend::new();
                VerificationTracer::new(backend, TraceEngineConfig::default())
            },
            |mut tracer| {
                black_box(
                    tracer
                        .log_read(ThreadId::new(0), Address::new(0x1000), 42)
                        .unwrap(),
                )
            },
            BatchSize::SmallInput,
        )
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 3: Causality Verification and Graph
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_tracing_causality(c: &mut Criterion) {
    let mut group = c.benchmark_group("tracing/causality");

    let event_counts = [10, 50]; // Verification limits max to ~64, keep within bounds

    for &n in &event_counts {
        group.bench_with_input(BenchmarkId::new("verify_causality/prod", n), &n, |b, &n| {
            let backend = fill_prod_backend(n);
            b.iter(|| black_box(backend.verify_causality().unwrap()))
        });

        group.bench_with_input(
            BenchmarkId::new("verify_causality/verif", n),
            &n,
            |b, &n| {
                let backend = fill_verif_backend(n);
                b.iter(|| black_box(backend.verify_causality().unwrap()))
            },
        );

        group.bench_with_input(BenchmarkId::new("causality_graph/prod", n), &n, |b, &n| {
            let backend = fill_prod_backend(n);
            let events = backend.get_all_events();
            b.iter(|| black_box(CausalityGraph::from_trace(black_box(events))))
        });
    }

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 4: FFI / Serialization Overhead
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// SimulationEvent는 순수 도메인 객체이므로 FFI 전송을 위한 로컬 래퍼를 생성합니다.
#[derive(serde::Serialize, serde::Deserialize)]
struct TracingSnapshot {
    event_count: usize,
    global_lamport: u64,
    is_causally_valid: bool,
}

fn bench_tracing_ffi_serialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("tracing/ffi_serialize");

    let snap = TracingSnapshot {
        event_count: 100_000,
        global_lamport: 42000,
        is_causally_valid: true,
    };

    group.bench_function("serialize_event", |b| {
        b.iter(|| black_box(serde_json::to_string(black_box(&snap)).unwrap()))
    });

    let json_snap = serde_json::to_string(&snap).unwrap();
    group.bench_function("deserialize_event", |b| {
        b.iter(|| {
            black_box(
                serde_json::from_str::<TracingSnapshot>(black_box(json_snap.as_str())).unwrap(),
            )
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

            black_box(serde_json::from_slice::<TracingSnapshot>(slice).unwrap())
        })
    });

    group.finish();
}

criterion_group!(
    name = tracing_benches;
    config = Criterion::default()
        .warm_up_time(std::time::Duration::from_millis(500))
        .measurement_time(std::time::Duration::from_secs(3));
    targets =
        bench_tracing_init,
        bench_tracing_logging,
        bench_tracing_causality,
        bench_tracing_ffi_serialize
);

criterion_main!(tracing_benches);
