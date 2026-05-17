//! # Context Benchmark Suite
//!
//! Measures pure data-shape and allocation performance of `SovereignContext`
//! and `ContextBuilder`. No business logic is exercised.
//!
//! ## Measurement Targets
//!
//! 1. **Creation (Light)** — empty metadata, baseline allocation cost.
//! 2. **Creation (Heavy)** — 100+ child contexts spawned from a single parent,
//!    validates the sub-1 ms total latency budget.
//! 3. **Builder Pattern Overhead** — full fluent-chain assembly + immutable build.
//! 4. **FFI Boundary Simulation** — JSON serialization into an 8-byte-aligned
//!    scratch buffer and subsequent deserialization, mimicking the Rust ↔ Deno
//!    FFI crossing cost.
//!
//! ## Alignment Guarantee
//!
//! All FFI mock buffers are backed by `Vec<u64>` (guaranteed 8-byte alignment)
//! to honour the `FFI_BUFFER_ALIGN = 8` contract defined in `laplace-interfaces`.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use laplace_core::domain::context::ContextBuilder;
use laplace_interfaces::{
    domain::{PriorityLevel, SovereignContext, TenantTier},
    FfiBuffer,
};

// ============================================================================
// 1. Creation (Light)
// ============================================================================

/// Minimal context — three short identifiers, no extra allocation.
///
/// This is the absolute floor: any regression here indicates overhead
/// introduced into the core `SovereignContext::new` path.
fn bench_creation_light(c: &mut Criterion) {
    c.bench_function("context/creation_light", |b| {
        b.iter(|| {
            black_box(SovereignContext::new(
                black_box("req-0001".to_string()),
                black_box("tenant-a".to_string()),
                black_box("trace-00000001".to_string()),
            ))
        })
    });
}

// ============================================================================
// 2. Creation (Heavy)
// ============================================================================

/// Simulates a heavy-context workload:
///
/// - The parent context carries UUID-length identifiers (~36 chars each).
/// - 100 child contexts are spawned from the parent in a tight loop.
///
/// Target: total wall time **≤ 1 ms** for the 100-context batch,
/// confirming that per-context overhead stays in the low-µs range.
fn bench_creation_heavy(c: &mut Criterion) {
    // UUID-length strings (36 chars) are representative of production IDs.
    let parent = SovereignContext::new(
        "a1b2c3d4-e5f6-7890-abcd-ef1234567890".to_string(),
        "tenant-enterprise-0000000000000001".to_string(),
        "00112233-4455-6677-8899-aabbccddeeff".to_string(),
    );

    let mut group = c.benchmark_group("context/creation_heavy");

    // Parameterised by batch size so the chart shows scaling behaviour.
    for n in [10usize, 50, 100, 200] {
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| {
                // Spawn n child contexts; each clones tenant/trace strings.
                let children: Vec<SovereignContext> = (0..n)
                    .map(|i| parent.spawn_child(black_box(format!("child-req-{:08}", i))))
                    .collect();
                black_box(children)
            })
        });
    }

    group.finish();
}

// ============================================================================
// 3. Builder Pattern Overhead
// ============================================================================

/// Measures the cost of assembling a `SovereignContext` via `ContextBuilder`.
///
/// Three sub-benchmarks isolate different builder configurations:
///
/// - `minimal` — just request/tenant/trace, all defaults.
/// - `full_standard` — every field set, standard FFI mode.
/// - `full_turbo` — every field set, turbo mode (includes auto-tier-upgrade
///   guard in the builder).
fn bench_builder_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("context/builder");

    // --- Minimal build ---
    group.bench_function("minimal", |b| {
        b.iter(|| {
            black_box(
                ContextBuilder::new(black_box("req-build-min"))
                    .tenant(black_box("tenant-min"))
                    .trace(black_box("trace-min"))
                    .build(),
            )
        })
    });

    // --- Full standard build (all setters, no turbo) ---
    group.bench_function("full_standard", |b| {
        b.iter(|| {
            black_box(
                ContextBuilder::new(black_box("req-build-full"))
                    .tenant(black_box("tenant-acme"))
                    .trace(black_box("trace-full-0000001"))
                    .priority(black_box(PriorityLevel::Critical))
                    .tier(black_box(TenantTier::Enterprise))
                    .turbo(black_box(false))
                    .build(),
            )
        })
    });

    // --- Full turbo build (triggers tier-compatibility guard) ---
    group.bench_function("full_turbo", |b| {
        b.iter(|| {
            black_box(
                ContextBuilder::new(black_box("req-build-turbo"))
                    .tenant(black_box("tenant-premium"))
                    .trace(black_box("trace-turbo-0000001"))
                    .priority(black_box(PriorityLevel::High))
                    .tier(black_box(TenantTier::Pro))
                    .turbo(black_box(true))
                    .build(),
            )
        })
    });

    group.finish();
}

// ============================================================================
// 4. FFI Boundary Simulation
// ============================================================================

/// Simulates the Rust ↔ Deno FFI serialization round-trip.
///
/// ## 8-byte Alignment
///
/// Mock buffers are backed by `Vec<u64>` whose element alignment is 8 bytes,
/// satisfying the `FFI_BUFFER_ALIGN = 8` requirement for 64-bit atomic
/// instruction boundaries and cache-line correctness.
///
/// The benchmark does **not** cross an actual FFI boundary; it measures the
/// pure serialization / deserialization overhead so that cost can be budgeted
/// separately from transport latency.
fn bench_ffi_boundary(c: &mut Criterion) {
    let ctx = SovereignContext::new(
        "req-ffi-benchmark-001".to_string(),
        "tenant-ffi-benchmark".to_string(),
        "trace-ffi-0000000001".to_string(),
    );

    let mut group = c.benchmark_group("context/ffi_boundary");

    // --- Serialization: SovereignContext → 8-byte-aligned byte buffer ---
    group.bench_function("serialize", |b| {
        b.iter(|| {
            // Step 1: JSON-encode the context (mirrors Protobuf/JSON FFI path).
            let bytes = serde_json::to_vec(black_box(&ctx)).expect("serialise");

            // Step 2: Allocate an 8-byte-aligned scratch buffer.
            //         Vec<u64> guarantees align_of::<u64>() == 8.
            let words = bytes.len().div_ceil(8);
            let mut aligned: Vec<u64> = vec![0u64; words];

            // Step 3: Copy payload into aligned storage.
            // SAFETY: `aligned` owns the allocation; we write `bytes.len()`
            //         bytes starting at offset 0, which is within capacity.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    aligned.as_mut_ptr().cast::<u8>(),
                    bytes.len(),
                );
            }

            // Step 4: Construct the FfiBuffer descriptor (32 bytes, align 8).
            let ffi_buf = FfiBuffer {
                data: aligned.as_mut_ptr().cast::<u8>(),
                len: bytes.len(),
                cap: words * 8,
                _padding: 0,
            };

            // Prevent the compiler from eliding the allocation/copy.
            black_box((ffi_buf, aligned))
        })
    });

    // --- Deserialization: 8-byte-aligned byte buffer → SovereignContext ---
    //
    // Pre-populate the aligned buffer outside the hot loop so the benchmark
    // measures *only* the deserialization cost.
    {
        let bytes = serde_json::to_vec(&ctx).expect("serialise");
        let words = bytes.len().div_ceil(8);
        let mut aligned: Vec<u64> = vec![0u64; words];
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                aligned.as_mut_ptr().cast::<u8>(),
                bytes.len(),
            );
        }
        let payload_len = bytes.len();

        group.bench_function("deserialize", |b| {
            b.iter(|| {
                // Reconstruct a byte slice from the aligned buffer.
                // SAFETY: `aligned` outlives this closure; `payload_len` bytes
                //         within the allocation are valid UTF-8 JSON.
                let slice = unsafe {
                    std::slice::from_raw_parts(aligned.as_ptr().cast::<u8>(), payload_len)
                };
                let restored: SovereignContext =
                    serde_json::from_slice(black_box(slice)).expect("deserialise");
                black_box(restored)
            })
        });
    }

    // --- Round-trip: serialize → aligned buffer → deserialize ---
    group.bench_function("roundtrip", |b| {
        b.iter(|| {
            let bytes = serde_json::to_vec(black_box(&ctx)).expect("serialise");
            let words = bytes.len().div_ceil(8);
            let mut aligned: Vec<u64> = vec![0u64; words];
            unsafe {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    aligned.as_mut_ptr().cast::<u8>(),
                    bytes.len(),
                );
            }
            let slice =
                unsafe { std::slice::from_raw_parts(aligned.as_ptr().cast::<u8>(), bytes.len()) };
            let restored: SovereignContext =
                serde_json::from_slice(black_box(slice)).expect("deserialise");
            black_box(restored)
        })
    });

    group.finish();
}

// ============================================================================
// Criterion entry points
// ============================================================================

criterion_group!(
    benches,
    bench_creation_light,
    bench_creation_heavy,
    bench_builder_overhead,
    bench_ffi_boundary,
);
criterion_main!(benches);
