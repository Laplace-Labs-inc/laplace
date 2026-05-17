// SPDX-License-Identifier: Apache-2.0
//! Pool Domain Benchmarks
//!
//! Measures performance characteristics of the `domain::pool` module:
//! - `PoolSnapshot` construction and computed metric overhead
//! - `PoolPolicy` pure decision function throughput (storage routing, preemption,
//!   eviction, throttle)
//! - `PoolHealthCheck::assess()` latency under healthy / degraded / unhealthy load
//! - FFI serialization round-trip (JSON ↔ 8-byte-aligned `Vec<u64>` mock buffer)
//!
//! **Zero-Implementation Rule**: This file calls only the existing pool module API.
//! No new business logic is introduced here.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use laplace_core::domain::pool::{
    HealthStatus, PoolHealthCheck, PoolPolicy, PoolSnapshot, StorageStrategy,
};
use laplace_interfaces::domain::TenantTier;
use std::time::{Duration, Instant};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// FFI alignment constant (matches laplace-interfaces FFI_BUFFER_ALIGN = 8)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

const FFI_BUFFER_ALIGN: usize = 8;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Fixture helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Healthy snapshot: moderate load on both Turbo and Standard paths.
fn make_healthy_snapshot() -> PoolSnapshot {
    PoolSnapshot {
        cached_isolates: 50,
        max_capacity: 100,
        healthy: true,
        turbo_active: 20,
        turbo_capacity: 50,
        turbo_utilization_pct: 40,
        turbo_fallback_count: 5,
        turbo_avg_reuse_count: 10,
        standard_active: 30,
        standard_capacity: 50,
        standard_utilization_pct: 60,
        standard_avg_lifetime_secs: 300,
    }
}

/// Degraded snapshot: Turbo utilization >85%, high fallback count.
fn make_degraded_snapshot() -> PoolSnapshot {
    PoolSnapshot {
        cached_isolates: 80,
        max_capacity: 100,
        healthy: false,
        turbo_active: 46,
        turbo_capacity: 50,
        turbo_utilization_pct: 92,
        turbo_fallback_count: 60,
        turbo_avg_reuse_count: 25,
        standard_active: 34,
        standard_capacity: 50,
        standard_utilization_pct: 68,
        standard_avg_lifetime_secs: 180,
    }
}

/// Near-capacity (unhealthy) snapshot: overall utilization ≥ 95%.
fn make_unhealthy_snapshot() -> PoolSnapshot {
    PoolSnapshot {
        cached_isolates: 97,
        max_capacity: 100,
        healthy: false,
        turbo_active: 50,
        turbo_capacity: 50,
        turbo_utilization_pct: 100,
        turbo_fallback_count: 500,
        turbo_avg_reuse_count: 50,
        standard_active: 47,
        standard_capacity: 50,
        standard_utilization_pct: 94,
        standard_avg_lifetime_secs: 30,
    }
}

/// All five tenant tiers for parameterised benchmarks.
const ALL_TIERS: [TenantTier; 5] = [
    TenantTier::Free,
    TenantTier::Standard,
    TenantTier::Turbo,
    TenantTier::Pro,
    TenantTier::Enterprise,
];

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 1: Initialization / PoolSnapshot Construction
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_pool_init(c: &mut Criterion) {
    let mut group = c.benchmark_group("pool/init");

    // Minimal default construction (zero-initialised via Default impl)
    group.bench_function("default", |b| b.iter(|| black_box(PoolSnapshot::default())));

    // Healthy snapshot with realistic field values
    group.bench_function("healthy", |b| b.iter(|| black_box(make_healthy_snapshot())));

    // Degraded snapshot — slightly more fields set to non-zero
    group.bench_function("degraded", |b| {
        b.iter(|| black_box(make_degraded_snapshot()))
    });

    // Near-capacity snapshot — all counters near maximum
    group.bench_function("unhealthy", |b| {
        b.iter(|| black_box(make_unhealthy_snapshot()))
    });

    // Recommended pool size lookup per tier (pure match, no allocation)
    for tier in ALL_TIERS {
        group.bench_with_input(
            BenchmarkId::new("recommended_size", format!("{:?}", tier)),
            &tier,
            |b, &tier| b.iter(|| black_box(PoolPolicy::recommended_pool_size(tier))),
        );
    }

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 2: Policy Decisions (Acquire / Release Simulation)
//
// The pool module contains pure decision functions instead of a traditional
// acquire/release object pool.  These benchmarks model the overhead of the
// routing, preemption, eviction, and throttle decisions made during each
// acquire/release cycle.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_pool_policy(c: &mut Criterion) {
    let mut group = c.benchmark_group("pool/policy");

    // Storage strategy routing per tier (hot path during every acquire)
    for tier in ALL_TIERS {
        group.bench_with_input(
            BenchmarkId::new("storage_strategy", format!("{:?}", tier)),
            &tier,
            |b, &tier| b.iter(|| black_box(PoolPolicy::determine_storage_strategy(tier))),
        );
    }

    // Allocation priority lookup per tier
    for tier in ALL_TIERS {
        group.bench_with_input(
            BenchmarkId::new("allocation_priority", format!("{:?}", tier)),
            &tier,
            |b, &tier| b.iter(|| black_box(PoolPolicy::allocation_priority(tier))),
        );
    }

    // Preemption check — best case: Enterprise can preempt Free (returns true)
    group.bench_function("can_preempt/enterprise_vs_free", |b| {
        b.iter(|| {
            black_box(PoolPolicy::can_preempt(
                TenantTier::Enterprise,
                TenantTier::Free,
            ))
        })
    });

    // Preemption check — same tier (returns false, short-circuit path)
    group.bench_function("can_preempt/standard_vs_standard", |b| {
        b.iter(|| {
            black_box(PoolPolicy::can_preempt(
                TenantTier::Standard,
                TenantTier::Standard,
            ))
        })
    });

    // Eviction decision: resource still fresh (returns false)
    group.bench_function("should_evict/not_evicted", |b| {
        let last_used = Instant::now();
        let max_idle = Duration::from_secs(300);
        b.iter(|| black_box(PoolPolicy::should_evict(last_used, max_idle)))
    });

    // Eviction decision: resource past threshold (returns true)
    group.bench_function("should_evict/evicted", |b| {
        let last_used = Instant::now() - Duration::from_secs(400);
        let max_idle = Duration::from_secs(300);
        b.iter(|| black_box(PoolPolicy::should_evict(last_used, max_idle)))
    });

    // Eviction threshold lookup per tier (pure match → Duration)
    for tier in ALL_TIERS {
        group.bench_with_input(
            BenchmarkId::new("eviction_threshold", format!("{:?}", tier)),
            &tier,
            |b, &tier| b.iter(|| black_box(PoolPolicy::eviction_threshold(tier))),
        );
    }

    // Throttle check: Free tier over limit (returns true)
    group.bench_function("should_throttle/free_throttled", |b| {
        b.iter(|| black_box(PoolPolicy::should_throttle(TenantTier::Free, 100)))
    });

    // Throttle check: Enterprise unlimited (returns false, u64::MAX comparison)
    group.bench_function("should_throttle/enterprise_not_throttled", |b| {
        b.iter(|| {
            black_box(PoolPolicy::should_throttle(
                TenantTier::Enterprise,
                1_000_000,
            ))
        })
    });

    // Preemption victim selection — scales O(N) with candidate pool size
    for n in [4usize, 16, 64, 256] {
        let now = Instant::now();
        let candidates: Vec<(TenantTier, Instant)> = (0..n)
            .map(|i| {
                let tier = ALL_TIERS[i % ALL_TIERS.len()];
                let last_used = now - Duration::from_millis((i as u64) * 10);
                (tier, last_used)
            })
            .collect();

        group.bench_with_input(
            BenchmarkId::new("select_victim", n),
            &candidates,
            |b, candidates| {
                b.iter(|| {
                    black_box(PoolPolicy::select_preemption_victim(
                        TenantTier::Enterprise,
                        candidates,
                    ))
                })
            },
        );
    }

    // StorageStrategy property accessors (used in monitoring/alerting hot paths)
    group.bench_function("strategy/expected_latency_standard", |b| {
        b.iter(|| black_box(StorageStrategy::Standard.expected_latency_ns()))
    });

    group.bench_function("strategy/expected_latency_turbo", |b| {
        b.iter(|| black_box(StorageStrategy::Turbo.expected_latency_ns()))
    });

    group.bench_function("strategy/is_zero_copy", |b| {
        b.iter(|| black_box(StorageStrategy::Turbo.is_zero_copy()))
    });

    group.bench_function("strategy/name", |b| {
        b.iter(|| black_box(StorageStrategy::Turbo.name()))
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 3: Health Assessment
//
// Models a monitoring thread periodically reading pool metrics and running the
// multi-condition health assessment.  Called once per scrape interval (typically
// every few seconds) but also inline during admission control checks.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_pool_health(c: &mut Criterion) {
    let mut group = c.benchmark_group("pool/health");

    let healthy = make_healthy_snapshot();
    let degraded = make_degraded_snapshot();
    let unhealthy = make_unhealthy_snapshot();

    // Health classification per scenario
    group.bench_function("assess/healthy", |b| {
        b.iter(|| black_box(PoolHealthCheck::assess(black_box(&healthy))))
    });

    group.bench_function("assess/degraded", |b| {
        b.iter(|| black_box(PoolHealthCheck::assess(black_box(&degraded))))
    });

    group.bench_function("assess/unhealthy", |b| {
        b.iter(|| black_box(PoolHealthCheck::assess(black_box(&unhealthy))))
    });

    // HealthStatus predicate methods (used in alerting / conditional branching)
    group.bench_function("status/is_healthy", |b| {
        let status = HealthStatus::Healthy;
        b.iter(|| black_box(status.is_healthy()))
    });

    group.bench_function("status/is_degraded", |b| {
        let status = HealthStatus::Degraded {
            reason: "Turbo pool under pressure".to_string(),
        };
        b.iter(|| black_box(status.is_degraded()))
    });

    group.bench_function("status/reason", |b| {
        let status = HealthStatus::Unhealthy {
            reason: "Pool near capacity".to_string(),
        };
        b.iter(|| black_box(status.reason()))
    });

    // Snapshot computed metrics (integer / float arithmetic over struct fields)
    group.bench_function("metrics/overall_utilization_pct", |b| {
        b.iter(|| black_box(healthy.overall_utilization_pct()))
    });

    group.bench_function("metrics/is_under_pressure", |b| {
        b.iter(|| black_box(healthy.is_under_pressure()))
    });

    group.bench_function("metrics/should_scale_turbo", |b| {
        b.iter(|| black_box(degraded.should_scale_turbo()))
    });

    group.bench_function("metrics/should_scale_standard", |b| {
        b.iter(|| black_box(degraded.should_scale_standard()))
    });

    group.bench_function("metrics/turbo_adoption_rate", |b| {
        b.iter(|| black_box(healthy.turbo_adoption_rate()))
    });

    group.bench_function("metrics/turbo_efficiency_score", |b| {
        b.iter(|| black_box(healthy.turbo_efficiency_score()))
    });

    group.bench_function("metrics/turbo_available", |b| {
        b.iter(|| black_box(healthy.turbo_available()))
    });

    group.bench_function("metrics/standard_available", |b| {
        b.iter(|| black_box(healthy.standard_available()))
    });

    group.bench_function("metrics/has_turbo_capacity", |b| {
        b.iter(|| black_box(healthy.has_turbo_capacity()))
    });

    group.bench_function("metrics/has_standard_capacity", |b| {
        b.iter(|| black_box(healthy.has_standard_capacity()))
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Group 4: FFI / Serialization Overhead
//
// PoolSnapshot derives Serialize + Deserialize.  These benchmarks measure the
// cost of transmitting a pool status report across the Rust ↔ V8 Standard FFI
// boundary (JSON wire format) and packing the bytes into a Vec<u64> buffer that
// satisfies the 8-byte alignment contract of laplace-interfaces.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_pool_ffi_serialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("pool/ffi_serialize");

    let healthy = make_healthy_snapshot();
    let unhealthy = make_unhealthy_snapshot();

    // JSON encode — default (minimal payload)
    group.bench_function("serialize_default", |b| {
        let snap = PoolSnapshot::default();
        b.iter(|| black_box(serde_json::to_string(black_box(&snap)).unwrap()))
    });

    // JSON encode — healthy snapshot (typical monitoring payload)
    group.bench_function("serialize_healthy", |b| {
        b.iter(|| black_box(serde_json::to_string(black_box(&healthy)).unwrap()))
    });

    // JSON encode — near-capacity snapshot (large numeric strings)
    group.bench_function("serialize_unhealthy", |b| {
        b.iter(|| black_box(serde_json::to_string(black_box(&unhealthy)).unwrap()))
    });

    // JSON decode — from pre-encoded string (avoids encode cost)
    let json_healthy = serde_json::to_string(&healthy).unwrap();
    group.bench_function("deserialize_healthy", |b| {
        b.iter(|| {
            black_box(
                serde_json::from_str::<PoolSnapshot>(black_box(json_healthy.as_str())).unwrap(),
            )
        })
    });

    // Full round-trip encode + decode
    group.bench_function("roundtrip_healthy", |b| {
        b.iter(|| {
            let encoded = serde_json::to_string(black_box(&healthy)).unwrap();
            black_box(serde_json::from_str::<PoolSnapshot>(&encoded).unwrap())
        })
    });

    // Pack JSON bytes into an 8-byte-aligned Vec<u64> (write direction)
    //
    // Vec<u64> guarantees align(8) — satisfies FFI_BUFFER_ALIGN contract.
    // Models the cost of staging a PoolSnapshot report for V8 consumption.
    group.bench_function("ffi_buffer_pack", |b| {
        b.iter(|| {
            let json = serde_json::to_string(black_box(&healthy)).unwrap();
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

    // Read back from an 8-byte-aligned Vec<u64> (read direction)
    let json_pack = serde_json::to_string(&healthy).unwrap();
    let pack_bytes = json_pack.as_bytes();
    let word_count = pack_bytes.len().div_ceil(FFI_BUFFER_ALIGN);
    let mut pack_buf: Vec<u64> = vec![0u64; word_count];
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
            // Safety: pack_buf contains valid UTF-8 JSON from serde_json above
            let slice = unsafe {
                std::slice::from_raw_parts(pack_buf.as_ptr() as *const u8, pack_byte_len)
            };
            let decoded: PoolSnapshot = serde_json::from_slice(black_box(slice)).unwrap();
            black_box(decoded)
        })
    });

    // Full FFI round-trip: encode → pack → unpack → decode
    group.bench_function("ffi_buffer_roundtrip", |b| {
        b.iter(|| {
            // Encode
            let json = serde_json::to_string(black_box(&healthy)).unwrap();
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
            // Unpack + Decode
            let slice = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, byte_len) };
            let decoded: PoolSnapshot = serde_json::from_slice(slice).unwrap();
            black_box(decoded)
        })
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Criterion entry point
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

criterion_group!(
    name = pool_benches;
    config = Criterion::default()
        .warm_up_time(std::time::Duration::from_millis(500))
        .measurement_time(std::time::Duration::from_secs(3));
    targets =
        bench_pool_init,
        bench_pool_policy,
        bench_pool_health,
        bench_pool_ffi_serialize
);

criterion_main!(pool_benches);
