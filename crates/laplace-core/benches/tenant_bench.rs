// SPDX-License-Identifier: Apache-2.0
//! Tenant Domain Benchmarks
//!
//! Measures performance characteristics of the `domain::tenant` module:
//! - `tenant/init`           – TenantMetadata allocation, creation cost, and Tier enums.
//! - `tenant/capabilities`   – Pure O(1) enum matching for tier capabilities and quotas.
//! - `tenant/ffi_serialize`  – JSON encode/decode of TenantMetadata and packing
//!   into an 8-byte-aligned `Vec<u64>` mock-FFI buffer.
//!
//! **Zero-Implementation Rule**: Validates that Multi-tenancy isolation checks
//! (like `uses_turbo` or `for_tier`) add virtually zero runtime overhead.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use laplace_interfaces::domain::{ResourceConfig, TenantMetadata, TenantTier};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// FFI alignment constant (matches laplace-interfaces FFI_BUFFER_ALIGN = 8)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
const FFI_BUFFER_ALIGN: usize = 8;

const ALL_TIERS: [TenantTier; 5] = [
    TenantTier::Free,
    TenantTier::Standard,
    TenantTier::Turbo,
    TenantTier::Pro,
    TenantTier::Enterprise,
];

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 1: Initialization
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_tenant_init(c: &mut Criterion) {
    let mut group = c.benchmark_group("tenant/init");

    // Measure the cost of creating TenantMetadata (String allocations + SystemTime::now)
    group.bench_function("metadata_new/free", |b| {
        b.iter(|| {
            black_box(TenantMetadata::new(
                black_box("tenant-free-001".to_string()),
                black_box(TenantTier::Free),
            ))
        })
    });

    group.bench_function("metadata_new/enterprise", |b| {
        b.iter(|| {
            black_box(TenantMetadata::new(
                black_box("tenant-enterprise-999".to_string()),
                black_box(TenantTier::Enterprise),
            ))
        })
    });

    // Enum clone/copy overhead (should be sub-nanosecond)
    group.bench_function("tier_clone", |b| {
        let tier = TenantTier::Turbo;
        b.iter(|| black_box(*black_box(&tier)))
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 2: Capabilities & Quota Resolution
//
// Models the hot-path admission control checks that occur on every API request.
// These should compile down to simple jump tables (O(1) match statements).
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_tenant_capabilities(c: &mut Criterion) {
    let mut group = c.benchmark_group("tenant/capabilities");

    // ResourceConfig generation cost per tier
    for tier in ALL_TIERS {
        group.bench_with_input(
            BenchmarkId::new("resource_config_for_tier", format!("{:?}", tier)),
            &tier,
            |b, &t| b.iter(|| black_box(ResourceConfig::for_tier(black_box(t)))),
        );
    }

    // Pure enum match tests: validating O(1) routing speed for Turbo check
    for tier in ALL_TIERS {
        group.bench_with_input(
            BenchmarkId::new("uses_turbo_acceleration", format!("{:?}", tier)),
            &tier,
            |b, &t| b.iter(|| black_box(black_box(t).uses_turbo_acceleration())),
        );
    }

    // Sentinel check overhead
    group.bench_function("has_sentinel_monitoring", |b| {
        b.iter(|| black_box(black_box(TenantTier::Enterprise).has_sentinel_monitoring()))
    });

    // Validating tier upgrade progression logic
    group.bench_function("can_upgrade_to/valid", |b| {
        b.iter(|| black_box(TenantTier::Free.can_upgrade_to(TenantTier::Pro)))
    });

    group.bench_function("can_upgrade_to/invalid", |b| {
        b.iter(|| black_box(TenantTier::Enterprise.can_upgrade_to(TenantTier::Standard)))
    });

    // Pre-allocated metadata property access (very hot path)
    let meta = TenantMetadata::new("hot-tenant".to_string(), TenantTier::Pro);
    group.bench_function("metadata_uses_turbo", |b| {
        b.iter(|| black_box(black_box(&meta).uses_turbo()))
    });
    group.bench_function("metadata_validate", |b| {
        b.iter(|| black_box(black_box(&meta).validate()))
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 3: FFI / Serialization Overhead
//
// Measures exporting TenantMetadata to V8 Isolate or Axiom Backend.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_tenant_ffi_serialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("tenant/ffi_serialize");

    let meta = TenantMetadata::new(
        "tenant-enterprise-0000000000000001".to_string(),
        TenantTier::Enterprise,
    );

    // ── JSON encode ──────────────────────────────────────────────────────────
    group.bench_function("serialize", |b| {
        b.iter(|| black_box(serde_json::to_string(black_box(&meta)).unwrap()))
    });

    // ── JSON decode ──────────────────────────────────────────────────────────
    let json_meta = serde_json::to_string(&meta).unwrap();
    group.bench_function("deserialize", |b| {
        b.iter(|| {
            black_box(
                serde_json::from_str::<TenantMetadata>(black_box(json_meta.as_str())).unwrap(),
            )
        })
    });

    // ── FFI buffer pack: JSON → Vec<u64> (8-byte-aligned) ────────────────────
    group.bench_function("ffi_buffer_pack", |b| {
        b.iter(|| {
            let json = serde_json::to_string(black_box(&meta)).unwrap();
            let bytes = json.as_bytes();
            let word_count = bytes.len().div_ceil(FFI_BUFFER_ALIGN);
            let mut buf: Vec<u64> = vec![0u64; word_count];

            // Safety: buf is word_count * 8 bytes >= bytes.len()
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

    // ── FFI buffer roundtrip ─────────────────────────────────────────────────
    group.bench_function("ffi_buffer_roundtrip", |b| {
        b.iter(|| {
            // Encode
            let json = serde_json::to_string(black_box(&meta)).unwrap();
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

            // Unpack & Decode
            let slice = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, byte_len) };

            black_box(serde_json::from_slice::<TenantMetadata>(slice).unwrap())
        })
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Criterion entry point
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

criterion_group!(
    name = tenant_benches;
    config = Criterion::default()
        .warm_up_time(std::time::Duration::from_millis(500))
        .measurement_time(std::time::Duration::from_secs(3));
    targets =
        bench_tenant_init,
        bench_tenant_capabilities,
        bench_tenant_ffi_serialize
);

criterion_main!(tenant_benches);
