// SPDX-License-Identifier: Apache-2.0
//! # Journal Benchmark Suite
//!
//! Measures pure in-memory allocation and serialization performance of
//! `TransactionLog` and `LogStatus`. No file I/O, no business logic.
//!
//! ## Measurement Groups
//!
//! | Group | Description |
//! |---|---|
//! | `journal/append_light` | Minimal log entry — status-transition metadata only |
//! | `journal/append_heavy` | Full-payload entry — turbo metadata + error info + long strings |
//! | `journal/batch_append` | Batch creation into `Vec` (N = 100 / 1_000 / 5_000) |
//! | `journal/status_ops` | `LogStatus` enum conversions (to_code / from_code / clone) |
//! | `journal/ffi_serialize` | JSON → 8-byte-aligned scratch buffer (simulate Axiom export) |
//!
//! ## Alignment Guarantee
//!
//! All FFI mock buffers are backed by `Vec<u64>` (align = 8), satisfying the
//! `FFI_BUFFER_ALIGN = 8` contract from `laplace-interfaces`.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use laplace_core::domain::journal::{LogStatus, TransactionLog};

// ============================================================================
// Shared fixtures — pre-built outside the hot loop so we measure exactly
// what each benchmark intends to measure.
// ============================================================================

/// Short realistic identifiers (comparable to an 8-char snowflake fragment).
const SHORT_REQ: &str = "req-0001";
const SHORT_TEN: &str = "tenant-a";
const SHORT_OP: &str = "op_tick";

/// UUID-length identifiers (36 chars) typical of production traffic.
const LONG_REQ: &str = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
const LONG_TEN: &str = "tenant-enterprise-0000000000000001";
const LONG_OP: &str = "execute_script::v8_isolate::tenant_tick::checkpoint";

/// A realistic error message (~120 chars) with embedded diagnostics.
const HEAVY_ERR: &str = "RuntimeError: V8 isolate exceeded memory quota (512MiB). \
                          OOM at heap allocation 0x7fff_dead_beef. Tenant evicted. \
                          Fallback to Standard FFI.";

// ============================================================================
// 1. Single Append — Light
//    Minimal TransactionLog: three short strings, one status enum, no
//    optional fields. Baseline for raw struct-construction cost.
// ============================================================================

fn bench_append_light(c: &mut Criterion) {
    let mut group = c.benchmark_group("journal/append_light");

    // 1a. Standard FFI log — no optional fields populated.
    group.bench_function("new_standard", |b| {
        b.iter(|| {
            black_box(TransactionLog::new(
                black_box(SHORT_REQ.to_string()),
                black_box(SHORT_TEN.to_string()),
                black_box(SHORT_OP.to_string()),
                black_box(LogStatus::Success),
            ))
        })
    });

    // 1b. Standard FFI log with a single duration annotation.
    //     `.with_duration()` moves the struct — tests inline mutation cost.
    group.bench_function("new_standard_with_duration", |b| {
        b.iter(|| {
            black_box(
                TransactionLog::new(
                    black_box(SHORT_REQ.to_string()),
                    black_box(SHORT_TEN.to_string()),
                    black_box(SHORT_OP.to_string()),
                    black_box(LogStatus::Running),
                )
                .with_duration(black_box(42)),
            )
        })
    });

    // 1c. Clone cost of a light entry — relevant for audit-trail fan-out.
    {
        let light = TransactionLog::new(
            SHORT_REQ.to_string(),
            SHORT_TEN.to_string(),
            SHORT_OP.to_string(),
            LogStatus::Success,
        );
        group.bench_function("clone_light", |b| {
            b.iter(|| black_box(black_box(&light).clone()))
        });
    }

    group.finish();
}

// ============================================================================
// 2. Single Append — Heavy
//    Full-payload TransactionLog: Turbo metadata, long UUID strings, error
//    annotation. Validates < 1 µs single-entry budget for the hot path.
// ============================================================================

fn bench_append_heavy(c: &mut Criterion) {
    let mut group = c.benchmark_group("journal/append_heavy");

    // 2a. Turbo entry — slot metadata allocated, no error.
    group.bench_function("new_turbo", |b| {
        b.iter(|| {
            black_box(TransactionLog::new_turbo(
                black_box(LONG_REQ.to_string()),
                black_box(LONG_TEN.to_string()),
                black_box(LONG_OP.to_string()),
                black_box(LogStatus::Success),
                black_box(42_usize),   // turbo_slot_index
                black_box(8192_usize), // turbo_memory_offset
            ))
        })
    });

    // 2b. Turbo entry with duration annotation.
    group.bench_function("new_turbo_with_duration", |b| {
        b.iter(|| {
            black_box(
                TransactionLog::new_turbo(
                    black_box(LONG_REQ.to_string()),
                    black_box(LONG_TEN.to_string()),
                    black_box(LONG_OP.to_string()),
                    black_box(LogStatus::Success),
                    black_box(42_usize),
                    black_box(8192_usize),
                )
                .with_duration(black_box(450)), // Turbo target: <500 ns
            )
        })
    });

    // 2c. Failed entry carrying a long error message + error code.
    //     Tests double-String allocation (error_message on top of IDs).
    group.bench_function("new_failed_with_error", |b| {
        b.iter(|| {
            black_box(
                TransactionLog::new(
                    black_box(LONG_REQ.to_string()),
                    black_box(LONG_TEN.to_string()),
                    black_box(LONG_OP.to_string()),
                    black_box(LogStatus::Failed),
                )
                .with_duration(black_box(41_500)) // Standard FFI ~41.5 µs
                .with_error(black_box(HEAVY_ERR.to_string()), black_box(Some(2004_i32))),
            )
        })
    });

    // 2d. TurboFallback entry — all optional fields populated (worst case).
    group.bench_function("new_turbo_fallback_full", |b| {
        b.iter(|| {
            black_box(
                TransactionLog::new_turbo(
                    black_box(LONG_REQ.to_string()),
                    black_box(LONG_TEN.to_string()),
                    black_box(LONG_OP.to_string()),
                    black_box(LogStatus::TurboFallback),
                    black_box(255_usize),
                    black_box(131_072_usize),
                )
                .with_duration(black_box(2_100))
                .with_error(black_box(HEAVY_ERR.to_string()), black_box(Some(5001_i32))),
            )
        })
    });

    // 2e. Clone of a heavy entry — fan-out audit cost.
    {
        let heavy = TransactionLog::new_turbo(
            LONG_REQ.to_string(),
            LONG_TEN.to_string(),
            LONG_OP.to_string(),
            LogStatus::TurboFallback,
            255,
            131_072,
        )
        .with_duration(2_100)
        .with_error(HEAVY_ERR.to_string(), Some(5001));
        group.bench_function("clone_heavy", |b| {
            b.iter(|| black_box(black_box(&heavy).clone()))
        });
    }

    group.finish();
}

// ============================================================================
// 3. Batch Append — Throughput
//    Bulk-create N TransactionLog entries into a Vec in a single loop.
//    Measures Vec reallocation patterns, per-entry String allocation, and
//    cache-line pressure from struct size.
//    N is parameterised: 100, 1_000, 5_000.
// ============================================================================

fn bench_batch_append(c: &mut Criterion) {
    let mut group = c.benchmark_group("journal/batch_append");

    // Cycle through all 8 LogStatus variants to avoid branch prediction bias.
    const ALL_STATUSES: [LogStatus; 8] = [
        LogStatus::Pending,
        LogStatus::Running,
        LogStatus::Success,
        LogStatus::Failed,
        LogStatus::Timeout,
        LogStatus::Cancelled,
        LogStatus::Evicted,
        LogStatus::TurboFallback,
    ];

    // 3a. Standard FFI batch — one String allocation per entry (op_name varies).
    for &n in &[100_usize, 1_000, 5_000] {
        group.bench_with_input(BenchmarkId::new("standard_ffi", n), &n, |b, &n| {
            b.iter(|| {
                let mut logs: Vec<TransactionLog> = Vec::with_capacity(n);
                for i in 0..n {
                    logs.push(TransactionLog::new(
                        black_box(format!("req-{:08}", i)),
                        black_box(SHORT_TEN.to_string()),
                        black_box(SHORT_OP.to_string()),
                        black_box(ALL_STATUSES[i % ALL_STATUSES.len()]),
                    ));
                }
                black_box(logs)
            })
        });
    }

    // 3b. Turbo batch — adds two usize fields (slot_index + memory_offset)
    //     per entry; tests the additional Option<usize> allocation overhead.
    for &n in &[100_usize, 1_000, 5_000] {
        group.bench_with_input(BenchmarkId::new("turbo", n), &n, |b, &n| {
            b.iter(|| {
                let mut logs: Vec<TransactionLog> = Vec::with_capacity(n);
                for i in 0..n {
                    logs.push(TransactionLog::new_turbo(
                        black_box(format!("req-{:08}", i)),
                        black_box(LONG_TEN.to_string()),
                        black_box(LONG_OP.to_string()),
                        black_box(LogStatus::Success),
                        black_box(i % 256), // slot_index cycles 0..255
                        black_box(i * 512), // memory_offset grows linearly
                    ));
                }
                black_box(logs)
            })
        });
    }

    // 3c. Pre-allocated batch drain — measure pure Vec<TransactionLog> append
    //     with a pre-built template (no String allocation in hot path).
    //     Models the case where entries are cloned from a prototype.
    let template = TransactionLog::new(
        SHORT_REQ.to_string(),
        SHORT_TEN.to_string(),
        SHORT_OP.to_string(),
        LogStatus::Success,
    );
    for &n in &[100_usize, 1_000, 5_000] {
        group.bench_with_input(BenchmarkId::new("clone_from_template", n), &n, |b, &n| {
            b.iter(|| {
                let logs: Vec<TransactionLog> =
                    (0..n).map(|_| black_box(template.clone())).collect();
                black_box(logs)
            })
        });
    }

    group.finish();
}

// ============================================================================
// 4. LogStatus Enum Operations
//    Pure enum overhead: to_code / from_code / classification predicates.
//    These are called on every journal entry at read time.
// ============================================================================

fn bench_status_ops(c: &mut Criterion) {
    let mut group = c.benchmark_group("journal/status_ops");

    // 4a. Status → numeric code (protobuf serialization path).
    group.bench_function("to_code_all_variants", |b| {
        b.iter(|| {
            let mut sum = 0u64;
            for code in 0u8..=7 {
                // Use from_code + to_code to cover both directions.
                if let Some(s) = LogStatus::from_code(black_box(code)) {
                    sum += s.to_code() as u64;
                }
            }
            black_box(sum)
        })
    });

    // 4b. Classification predicates — called during metric aggregation.
    group.bench_function("predicates", |b| {
        let statuses = [
            LogStatus::Pending,
            LogStatus::Running,
            LogStatus::Success,
            LogStatus::Failed,
            LogStatus::Timeout,
            LogStatus::Cancelled,
            LogStatus::Evicted,
            LogStatus::TurboFallback,
        ];
        b.iter(|| {
            let mut flags = 0u32;
            for &s in &statuses {
                flags |= s.is_terminal() as u32;
                flags |= (s.is_success() as u32) << 1;
                flags |= (s.is_failure() as u32) << 2;
                flags |= (s.is_in_progress() as u32) << 3;
                flags |= (s.is_turbo_related() as u32) << 4;
            }
            black_box(flags)
        })
    });

    // 4c. Name string retrieval — used in structured logging.
    group.bench_function("name_all_variants", |b| {
        let statuses = [
            LogStatus::Pending,
            LogStatus::Running,
            LogStatus::Success,
            LogStatus::Failed,
            LogStatus::Timeout,
            LogStatus::Cancelled,
            LogStatus::Evicted,
            LogStatus::TurboFallback,
        ];
        b.iter(|| {
            let mut len = 0usize;
            for &s in &statuses {
                len += black_box(s.name()).len();
            }
            black_box(len)
        })
    });

    // 4d. `Copy` clone — confirms zero-cost copy semantics for queue dispatch.
    group.bench_function("copy", |b| {
        let s = LogStatus::Success;
        b.iter(|| black_box(black_box(s)))
    });

    group.finish();
}

// ============================================================================
// 5. FFI / Serialization Overhead
//    JSON-encode a TransactionLog into an 8-byte-aligned scratch buffer, then
//    decode it back. Simulates the Axiom verification backend export path.
//
//    8-byte alignment: backing store is Vec<u64> (align_of = 8), satisfying
//    the FFI_BUFFER_ALIGN = 8 contract from laplace-interfaces.
// ============================================================================

fn bench_ffi_serialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("journal/ffi_serialize");

    // Pre-build fixture logs outside the hot loop.
    let light_log = TransactionLog::new(
        SHORT_REQ.to_string(),
        SHORT_TEN.to_string(),
        SHORT_OP.to_string(),
        LogStatus::Success,
    )
    .with_duration(42);

    let heavy_log = TransactionLog::new_turbo(
        LONG_REQ.to_string(),
        LONG_TEN.to_string(),
        LONG_OP.to_string(),
        LogStatus::TurboFallback,
        255,
        131_072,
    )
    .with_duration(2_100)
    .with_error(HEAVY_ERR.to_string(), Some(5001));

    // Helper: pack `bytes` into a Vec<u64>-backed aligned buffer.
    // Returns (aligned_buf, payload_len) so the caller can derive a &[u8].
    let pack_aligned = |bytes: &[u8]| -> (Vec<u64>, usize) {
        let words = bytes.len().div_ceil(8);
        let mut buf = vec![0u64; words];
        // SAFETY: buf owns the allocation; we write exactly bytes.len() bytes
        //         starting at offset 0, which is within the allocated capacity.
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                buf.as_mut_ptr().cast::<u8>(),
                bytes.len(),
            );
        }
        (buf, bytes.len())
    };

    // ── 5a. Serialization: light log → aligned buffer ──────────────────────
    group.bench_function("serialize_light", |b| {
        b.iter(|| {
            let bytes = serde_json::to_vec(black_box(&light_log)).expect("serialize");
            let (buf, len) = pack_aligned(&bytes);
            black_box((buf, len))
        })
    });

    // ── 5b. Serialization: heavy log → aligned buffer ──────────────────────
    group.bench_function("serialize_heavy", |b| {
        b.iter(|| {
            let bytes = serde_json::to_vec(black_box(&heavy_log)).expect("serialize");
            let (buf, len) = pack_aligned(&bytes);
            black_box((buf, len))
        })
    });

    // ── 5c. Deserialization: aligned buffer → TransactionLog (light) ───────
    {
        let bytes = serde_json::to_vec(&light_log).expect("serialize");
        let (aligned_light, light_len) = pack_aligned(&bytes);

        group.bench_function("deserialize_light", |b| {
            b.iter(|| {
                // SAFETY: `aligned_light` outlives this closure; `light_len` bytes
                //         at the start of the allocation contain valid UTF-8 JSON.
                let slice = unsafe {
                    std::slice::from_raw_parts(aligned_light.as_ptr().cast::<u8>(), light_len)
                };
                let log: TransactionLog =
                    serde_json::from_slice(black_box(slice)).expect("deserialize");
                black_box(log)
            })
        });
    }

    // ── 5d. Deserialization: aligned buffer → TransactionLog (heavy) ───────
    {
        let bytes = serde_json::to_vec(&heavy_log).expect("serialize");
        let (aligned_heavy, heavy_len) = pack_aligned(&bytes);

        group.bench_function("deserialize_heavy", |b| {
            b.iter(|| {
                let slice = unsafe {
                    std::slice::from_raw_parts(aligned_heavy.as_ptr().cast::<u8>(), heavy_len)
                };
                let log: TransactionLog =
                    serde_json::from_slice(black_box(slice)).expect("deserialize");
                black_box(log)
            })
        });
    }

    // ── 5e. Full round-trip: light log (alloc + serialize + deserialize) ────
    group.bench_function("roundtrip_light", |b| {
        b.iter(|| {
            let bytes = serde_json::to_vec(black_box(&light_log)).expect("serialize");
            let (buf, len) = pack_aligned(&bytes);
            let slice = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), len) };
            let log: TransactionLog =
                serde_json::from_slice(black_box(slice)).expect("deserialize");
            black_box(log)
        })
    });

    // ── 5f. Full round-trip: heavy log ─────────────────────────────────────
    group.bench_function("roundtrip_heavy", |b| {
        b.iter(|| {
            let bytes = serde_json::to_vec(black_box(&heavy_log)).expect("serialize");
            let (buf, len) = pack_aligned(&bytes);
            let slice = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), len) };
            let log: TransactionLog =
                serde_json::from_slice(black_box(slice)).expect("deserialize");
            black_box(log)
        })
    });

    // ── 5g. LogStatus code-path serialization (protobuf-style numeric codes) ─
    //        Tests the to_code / from_code cycle used in Protobuf FFI export.
    group.bench_function("status_code_roundtrip_batch", |b| {
        let statuses = [
            LogStatus::Pending,
            LogStatus::Running,
            LogStatus::Success,
            LogStatus::Failed,
            LogStatus::Timeout,
            LogStatus::Cancelled,
            LogStatus::Evicted,
            LogStatus::TurboFallback,
        ];
        b.iter(|| {
            let mut codes = [0u8; 8];
            for (i, &s) in statuses.iter().enumerate() {
                codes[i] = s.to_code();
            }
            // Simulate re-parsing at the receiving end.
            let mut parsed = [LogStatus::Pending; 8];
            for (i, &code) in codes.iter().enumerate() {
                parsed[i] = LogStatus::from_code(black_box(code)).unwrap();
            }
            black_box(parsed)
        })
    });

    group.finish();
}

// ============================================================================
// Criterion entry points
// ============================================================================

criterion_group!(
    benches,
    bench_append_light,
    bench_append_heavy,
    bench_batch_append,
    bench_status_ops,
    bench_ffi_serialize,
);
criterion_main!(benches);
