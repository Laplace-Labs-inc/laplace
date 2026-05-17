//! # Memory Benchmark Suite
//!
//! Measures pure in-memory allocation, access-pattern, and snapshot performance
//! for `domain::memory`. No file I/O or OS-level `mmap` operations are exercised.
//!
//! ## Measurement Groups
//!
//! | Group | Description |
//! |---|---|
//! | `memory/init` | Backend and `SimulatedMemory` construction cost |
//! | `memory/read_write` | Main-memory and store-buffer single-op latency |
//! | `memory/snapshot` | DPOR rollback: full-state capture + restore overhead |
//! | `memory/ffi_serialize` | State-metadata export to JSON + 8-byte-aligned buffer |
//!
//! ## 8-Byte Alignment Guarantee
//!
//! All FFI mock buffers are backed by `Vec<u64>` (align = 8), matching the
//! `FFI_BUFFER_ALIGN = 8` contract from `laplace-interfaces`.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use laplace_core::domain::memory::{
    Address, CoreId, MemoryBackend, MemoryConfig, ProductionBackend, SimulatedMemory, StoreEntry,
    Value, VerificationBackend,
};
use laplace_core::domain::time::{ProductionBackend as ClockProd, VirtualClock};

// ============================================================================
// Serialisable snapshot type used by the FFI benchmarks.
//
// The `domain::memory` types intentionally do not derive `Serialize` (they are
// pure domain primitives). We define a thin wrapper here so we can measure the
// JSON export cost without touching domain code.
// ============================================================================

#[derive(serde::Serialize, serde::Deserialize)]
struct MemSnapshot {
    num_cores: usize,
    max_buffer_size: usize,
    /// (address, value) pairs captured from main memory
    main: Vec<(usize, u64)>,
    /// (core_id, address, value) pending buffer entries
    buffers: Vec<(usize, usize, u64)>,
}

// ============================================================================
// Helpers: build backends and SimulatedMemory without noise
// ============================================================================

fn make_prod(num_cores: usize, buf_size: usize) -> ProductionBackend {
    ProductionBackend::new(num_cores, buf_size)
}

fn make_veri() -> VerificationBackend {
    VerificationBackend::new()
}

/// Build a `SimulatedMemory` backed by `ProductionBackend`.
fn make_simulated(
    num_cores: usize,
    buf_size: usize,
) -> SimulatedMemory<ProductionBackend, ClockProd> {
    let backend = ProductionBackend::new(num_cores, buf_size);
    let clock = VirtualClock::new(ClockProd::new());
    let config = MemoryConfig {
        num_cores,
        max_buffer_size: buf_size,
        ..MemoryConfig::default()
    };
    SimulatedMemory::new(backend, clock, config)
}

/// Populate `n` distinct addresses in the production backend's main memory.
fn populate_main(backend: &mut ProductionBackend, n: usize) {
    for i in 0..n {
        backend.write_main(Address::new(i), Value::new(i as u64 * 7 + 1));
    }
}

// ============================================================================
// 1. Initialization — Light
//    Measures pure construction cost: heap allocation, VecDeque allocation for
//    store buffers (ProductionBackend), and fixed-array initialisation
//    (VerificationBackend).
// ============================================================================

fn bench_init(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory/init");

    // 1a. ProductionBackend — 2 cores, 8-entry store buffers.
    //     Allocates a DashMap + 2 × RwLock<VecDeque>.
    group.bench_function("production_2c_8b", |b| {
        b.iter(|| black_box(ProductionBackend::new(black_box(2), black_box(8))))
    });

    // 1b. ProductionBackend — 4 cores, 256-entry store buffers (larger Vec).
    group.bench_function("production_4c_256b", |b| {
        b.iter(|| black_box(ProductionBackend::new(black_box(4), black_box(256))))
    });

    // 1c. VerificationBackend — fixed-size arrays on the stack.
    //     No heap allocation; baseline for symbolic-execution backends.
    group.bench_function("verification", |b| {
        b.iter(|| black_box(VerificationBackend::new()))
    });

    // 1d. SimulatedMemory (production backend + virtual clock).
    //     Measures the full system assembly cost including VirtualClock init.
    group.bench_function("simulated_production", |b| {
        b.iter(|| black_box(make_simulated(2, 8)))
    });

    // 1e. Reset (clear_all) — used as a lightweight "re-init" between runs.
    {
        let mut backend = make_prod(4, 256);
        populate_main(&mut backend, 256);
        group.bench_function("clear_all_production", |b| {
            b.iter(|| black_box(backend.clear_all()))
        });
    }

    group.finish();
}

// ============================================================================
// 2. Read/Write Ops — Throughput
//    Single-operation latency for the two backends and the high-level
//    SimulatedMemory API. Parameterised to expose both buffer hit/miss and
//    main-memory hot/cold paths.
// ============================================================================

fn bench_read_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory/read_write");

    // ── ProductionBackend: main memory ──────────────────────────────────────

    // 2a. Single write to main memory (DashMap insert).
    {
        let mut backend = make_prod(2, 64);
        group.bench_function("main_write/production", |b| {
            b.iter(|| {
                black_box(
                    backend.write_main(black_box(Address::new(0x1000)), black_box(Value::new(42))),
                )
            })
        });
    }

    // 2b. Read after write (hot cache line in DashMap).
    {
        let mut backend = make_prod(2, 64);
        backend.write_main(Address::new(0x1000), Value::new(42));
        group.bench_function("main_read_hot/production", |b| {
            b.iter(|| black_box(backend.read_main(black_box(Address::new(0x1000)))))
        });
    }

    // 2c. Read of unwritten address (returns zero; cold path).
    {
        let backend = make_prod(2, 64);
        group.bench_function("main_read_cold/production", |b| {
            b.iter(|| black_box(backend.read_main(black_box(Address::new(0xDEAD)))))
        });
    }

    // ── VerificationBackend: main memory ────────────────────────────────────

    // 2d. Single write to verification backend (array index + UnsafeCell).
    {
        let mut backend = make_veri();
        group.bench_function("main_write/verification", |b| {
            b.iter(|| {
                black_box(backend.write_main(black_box(Address::new(0)), black_box(Value::new(99))))
            })
        });
    }

    // 2e. Read from verification backend (array index read).
    {
        let mut backend = make_veri();
        backend.write_main(Address::new(0), Value::new(99));
        group.bench_function("main_read/verification", |b| {
            b.iter(|| black_box(backend.read_main(black_box(Address::new(0)))))
        });
    }

    // ── ProductionBackend: store buffers ─────────────────────────────────────

    // 2f. buffer_push — append one entry to a core's FIFO queue.
    {
        let mut backend = make_prod(2, 1024);
        let core = CoreId::new(0);
        let entry = StoreEntry::new(Address::new(0x100), Value::new(7));
        group.bench_function("buffer_push/production", |b| {
            b.iter(|| {
                // Drain first to avoid "buffer full" on repeated runs.
                let _ = backend.buffer_pop(core);
                black_box(backend.buffer_push(black_box(core), black_box(entry)))
            })
        });
    }

    // 2g. buffer_pop — dequeue the oldest entry.
    {
        let mut backend = make_prod(2, 1024);
        let core = CoreId::new(0);
        // Keep the buffer non-empty by re-pushing after each pop.
        let entry = StoreEntry::new(Address::new(0x200), Value::new(13));
        backend.buffer_push(core, entry).unwrap();
        group.bench_function("buffer_pop/production", |b| {
            b.iter(|| {
                let result = black_box(backend.buffer_pop(black_box(core)));
                // Re-push so the buffer is never empty between iterations.
                let _ = backend.buffer_push(core, entry);
                result
            })
        });
    }

    // 2h. buffer_lookup — load forwarding: address found in buffer (hit).
    {
        let mut backend = make_prod(2, 64);
        let core = CoreId::new(0);
        let addr = Address::new(0x300);
        backend
            .buffer_push(core, StoreEntry::new(addr, Value::new(55)))
            .unwrap();
        group.bench_function("buffer_lookup_hit/production", |b| {
            b.iter(|| black_box(backend.buffer_lookup(black_box(core), black_box(addr))))
        });
    }

    // 2i. buffer_lookup — address NOT in buffer (miss; falls through to main).
    {
        let backend = make_prod(2, 64);
        let core = CoreId::new(0);
        let addr = Address::new(0xBEEF);
        group.bench_function("buffer_lookup_miss/production", |b| {
            b.iter(|| black_box(backend.buffer_lookup(black_box(core), black_box(addr))))
        });
    }

    // ── SimulatedMemory: high-level API ─────────────────────────────────────

    // 2j. SimulatedMemory::write — buffer push + virtual-clock event schedule.
    {
        let mut mem = make_simulated(2, 64);
        let core = CoreId::new(0);
        let addr = Address::new(0x400);
        group.bench_function("simulated_write", |b| {
            b.iter(|| {
                // Pop any pending entry first to keep the buffer from filling.
                let _ = mem.flush_one(core);
                black_box(mem.write(black_box(core), black_box(addr), black_box(Value::new(1))))
            })
        });
    }

    // 2k. SimulatedMemory::read — forwarded from buffer (buffer hit).
    {
        let mut mem = make_simulated(2, 64);
        let core = CoreId::new(0);
        let addr = Address::new(0x500);
        mem.write(core, addr, Value::new(7)).unwrap();
        group.bench_function("simulated_read_forwarded", |b| {
            b.iter(|| black_box(mem.read(black_box(core), black_box(addr))))
        });
    }

    // 2l. SimulatedMemory::read — buffer miss, falls through to main memory.
    {
        let mut mem = make_simulated(2, 64);
        let addr = Address::new(0x600);
        // Write directly to main memory via backend (bypasses buffer).
        mem.backend_mut().write_main(addr, Value::new(42));
        let core = CoreId::new(0);
        group.bench_function("simulated_read_main_fallback", |b| {
            b.iter(|| black_box(mem.read(black_box(core), black_box(addr))))
        });
    }

    group.finish();
}

// ============================================================================
// 3. Snapshot & Restore — Heavy
//    Models DPOR state rollback: capturing all written addresses into a Vec
//    and restoring them. Parameterised by working-set size N.
//
//    For `VerificationBackend` (the actual DPOR backend), we capture the 4
//    bounded main-memory slots plus the 2 bounded store-buffer slots, then
//    restore them — the complete state is < 300 bytes.
// ============================================================================

fn bench_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory/snapshot");

    // ── ProductionBackend: capture N addresses → Vec, then restore ──────────
    for &n in &[10_usize, 100, 1_000] {
        // Build a pre-populated backend outside the hot loop.
        let mut prefilled = make_prod(4, 1024);
        populate_main(&mut prefilled, n);

        // Addresses we'll capture (deterministic, reproducible).
        let addrs: Vec<Address> = (0..n).map(Address::new).collect();

        group.bench_with_input(BenchmarkId::new("capture_production", n), &n, |b, _| {
            b.iter(|| {
                // Step 1: capture
                let snapshot: Vec<(Address, Value)> = addrs
                    .iter()
                    .map(|&a| (a, black_box(prefilled.read_main(a))))
                    .collect();
                black_box(snapshot)
            })
        });

        group.bench_with_input(BenchmarkId::new("restore_production", n), &n, |b, _| {
            // Build one snapshot before the hot loop.
            let snapshot: Vec<(Address, Value)> =
                addrs.iter().map(|&a| (a, prefilled.read_main(a))).collect();

            let mut dst = make_prod(4, 1024);
            b.iter(|| {
                for &(addr, val) in black_box(&snapshot) {
                    dst.write_main(addr, val);
                }
                black_box(())
            })
        });

        group.bench_with_input(
            BenchmarkId::new("capture_restore_roundtrip_production", n),
            &n,
            |b, _| {
                let mut backend = make_prod(4, 1024);
                populate_main(&mut backend, n);
                let addrs_inner: Vec<Address> = (0..n).map(Address::new).collect();

                b.iter(|| {
                    // Capture
                    let snap: Vec<(Address, Value)> = addrs_inner
                        .iter()
                        .map(|&a| (a, backend.read_main(a)))
                        .collect();
                    // Restore: clear and rewrite
                    backend.clear_all();
                    for &(addr, val) in &snap {
                        backend.write_main(addr, val);
                    }
                    black_box(())
                })
            },
        );
    }

    // ── VerificationBackend: full bounded-state snapshot + restore ───────────
    //    MAX_ADDRESSES = 4, MAX_CORES = 2, MAX_BUFFER_ENTRIES = 2 per core.
    //    Total state ≈ 300 bytes — the real DPOR rollback unit.
    {
        let veri_addrs = [
            Address::new(0),
            Address::new(1),
            Address::new(2),
            Address::new(3),
        ];
        let veri_cores = [CoreId::new(0), CoreId::new(1)];

        group.bench_function("capture_verification", |b| {
            let mut vback = make_veri();
            // Populate known state
            vback.write_main(Address::new(0), Value::new(10));
            vback.write_main(Address::new(1), Value::new(20));
            vback.write_main(Address::new(2), Value::new(30));
            vback.write_main(Address::new(3), Value::new(40));
            vback
                .buffer_push(
                    CoreId::new(0),
                    StoreEntry::new(Address::new(0), Value::new(99)),
                )
                .ok();
            vback
                .buffer_push(
                    CoreId::new(1),
                    StoreEntry::new(Address::new(1), Value::new(88)),
                )
                .ok();

            b.iter(|| {
                // Main memory snapshot (4 reads)
                let main: Vec<(Address, Value)> = veri_addrs
                    .iter()
                    .map(|&a| (a, black_box(vback.read_main(a))))
                    .collect();
                // Buffer snapshot: length + most-recent value per (core, addr)
                let bufs: Vec<(CoreId, usize)> = veri_cores
                    .iter()
                    .map(|&c| (c, black_box(vback.buffer_len(c))))
                    .collect();
                black_box((main, bufs))
            })
        });

        group.bench_function("restore_verification", |b| {
            // Static snapshot data (pre-computed, represents the state we roll back to).
            let main_snap: Vec<(Address, Value)> = veri_addrs
                .iter()
                .copied()
                .zip([10u64, 20, 30, 40].iter().copied().map(Value::new))
                .collect();
            let buf_snap: Vec<(CoreId, Address, Value)> = vec![
                (CoreId::new(0), Address::new(0), Value::new(99)),
                (CoreId::new(1), Address::new(1), Value::new(88)),
            ];

            let mut vback = make_veri();
            b.iter(|| {
                vback.clear_all();
                // Restore main memory
                for &(addr, val) in black_box(&main_snap) {
                    vback.write_main(addr, val);
                }
                // Restore store buffers
                for &(core, addr, val) in black_box(&buf_snap) {
                    let _ = vback.buffer_push(core, StoreEntry::new(addr, val));
                }
                black_box(())
            })
        });
    }

    group.finish();
}

// ============================================================================
// 4. FFI / Serialization Overhead
//    Exports a `MemSnapshot` (memory state metadata) to JSON, packs the bytes
//    into a Vec<u64>-backed 8-byte-aligned scratch buffer, then deserialises.
//    Models the path taken when Axiom exports memory state for verification.
//
//    8-byte alignment: Vec<u64> guarantees align_of::<u64>() == 8.
// ============================================================================

fn bench_ffi_serialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory/ffi_serialize");

    // Helper: pack `bytes` into a Vec<u64> (8-byte aligned) buffer.
    let pack = |bytes: &[u8]| -> (Vec<u64>, usize) {
        let words = bytes.len().div_ceil(8);
        let mut buf = vec![0u64; words];
        // SAFETY: `buf` owns the allocation; we write `bytes.len()` bytes
        //         starting at offset 0, staying within the allocated capacity.
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                buf.as_mut_ptr().cast::<u8>(),
                bytes.len(),
            );
        }
        (buf, bytes.len())
    };

    // ── Pre-build fixtures ───────────────────────────────────────────────────

    // Small snapshot: config-only, no memory entries (light export).
    let snap_empty = MemSnapshot {
        num_cores: 2,
        max_buffer_size: 8,
        main: vec![],
        buffers: vec![],
    };

    // Medium snapshot: 64 main-memory entries + 4 buffer entries.
    let snap_medium = MemSnapshot {
        num_cores: 4,
        max_buffer_size: 64,
        main: (0u64..64).map(|i| (i as usize, i * 7 + 1)).collect(),
        buffers: (0u64..4)
            .map(|i| (i as usize % 4, (i * 16) as usize, i * 3 + 1))
            .collect(),
    };

    // Large snapshot: 1024 main-memory entries (production-scale).
    let snap_large = MemSnapshot {
        num_cores: 4,
        max_buffer_size: 256,
        main: (0u64..1024).map(|i| (i as usize, i * 7 + 1)).collect(),
        buffers: (0u64..16)
            .map(|i| (i as usize % 4, (i * 64) as usize, i * 5 + 1))
            .collect(),
    };

    // ── 4a. Serialisation benchmarks ─────────────────────────────────────────

    group.bench_function("serialize_empty", |b| {
        b.iter(|| {
            let bytes = serde_json::to_vec(black_box(&snap_empty)).expect("serialize");
            let (buf, len) = pack(&bytes);
            black_box((buf, len))
        })
    });

    group.bench_function("serialize_medium", |b| {
        b.iter(|| {
            let bytes = serde_json::to_vec(black_box(&snap_medium)).expect("serialize");
            let (buf, len) = pack(&bytes);
            black_box((buf, len))
        })
    });

    group.bench_function("serialize_large", |b| {
        b.iter(|| {
            let bytes = serde_json::to_vec(black_box(&snap_large)).expect("serialize");
            let (buf, len) = pack(&bytes);
            black_box((buf, len))
        })
    });

    // ── 4b. Deserialisation benchmarks (aligned buffer → MemSnapshot) ────────

    {
        let bytes_empty = serde_json::to_vec(&snap_empty).expect("serialize");
        let bytes_medium = serde_json::to_vec(&snap_medium).expect("serialize");
        let bytes_large = serde_json::to_vec(&snap_large).expect("serialize");

        let (aligned_empty, len_empty) = pack(&bytes_empty);
        let (aligned_medium, len_medium) = pack(&bytes_medium);
        let (aligned_large, len_large) = pack(&bytes_large);

        group.bench_function("deserialize_empty", |b| {
            b.iter(|| {
                // SAFETY: `aligned_empty` outlives this closure; `len_empty` bytes
                //         contain valid UTF-8 JSON.
                let slice = unsafe {
                    std::slice::from_raw_parts(aligned_empty.as_ptr().cast::<u8>(), len_empty)
                };
                let snap: MemSnapshot =
                    serde_json::from_slice(black_box(slice)).expect("deserialize");
                black_box(snap)
            })
        });

        group.bench_function("deserialize_medium", |b| {
            b.iter(|| {
                let slice = unsafe {
                    std::slice::from_raw_parts(aligned_medium.as_ptr().cast::<u8>(), len_medium)
                };
                let snap: MemSnapshot =
                    serde_json::from_slice(black_box(slice)).expect("deserialize");
                black_box(snap)
            })
        });

        group.bench_function("deserialize_large", |b| {
            b.iter(|| {
                let slice = unsafe {
                    std::slice::from_raw_parts(aligned_large.as_ptr().cast::<u8>(), len_large)
                };
                let snap: MemSnapshot =
                    serde_json::from_slice(black_box(slice)).expect("deserialize");
                black_box(snap)
            })
        });
    }

    // ── 4c. Full round-trips ──────────────────────────────────────────────────

    group.bench_function("roundtrip_empty", |b| {
        b.iter(|| {
            let bytes = serde_json::to_vec(black_box(&snap_empty)).expect("ser");
            let (buf, len) = pack(&bytes);
            let slice = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), len) };
            let snap: MemSnapshot = serde_json::from_slice(black_box(slice)).expect("de");
            black_box(snap)
        })
    });

    group.bench_function("roundtrip_medium", |b| {
        b.iter(|| {
            let bytes = serde_json::to_vec(black_box(&snap_medium)).expect("ser");
            let (buf, len) = pack(&bytes);
            let slice = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), len) };
            let snap: MemSnapshot = serde_json::from_slice(black_box(slice)).expect("de");
            black_box(snap)
        })
    });

    group.bench_function("roundtrip_large", |b| {
        b.iter(|| {
            let bytes = serde_json::to_vec(black_box(&snap_large)).expect("ser");
            let (buf, len) = pack(&bytes);
            let slice = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), len) };
            let snap: MemSnapshot = serde_json::from_slice(black_box(slice)).expect("de");
            black_box(snap)
        })
    });

    // ── 4d. Live-capture + serialize (realistic Axiom export path) ────────────
    //        Extract state from a live backend, build a MemSnapshot, serialise.
    {
        let mut live = make_prod(4, 256);
        populate_main(&mut live, 64);
        let live_addrs: Vec<Address> = (0..64).map(Address::new).collect();
        let live_core0 = CoreId::new(0);
        live.buffer_push(live_core0, StoreEntry::new(Address::new(0), Value::new(1)))
            .ok();
        live.buffer_push(live_core0, StoreEntry::new(Address::new(1), Value::new(2)))
            .ok();

        group.bench_function("live_capture_and_serialize", |b| {
            b.iter(|| {
                // Step 1: extract state from backend (read 64 addresses)
                let main: Vec<(usize, u64)> = live_addrs
                    .iter()
                    .map(|&a| (a.as_usize(), live.read_main(a).as_u64()))
                    .collect();

                // Step 2: build snapshot struct
                let snap = MemSnapshot {
                    num_cores: live.num_cores(),
                    max_buffer_size: live.max_buffer_size(),
                    main,
                    buffers: vec![],
                };

                // Step 3: serialize + pack into 8-byte-aligned buffer
                let bytes = serde_json::to_vec(black_box(&snap)).expect("ser");
                let (buf, len) = pack(&bytes);
                black_box((buf, len))
            })
        });
    }

    group.finish();
}

// ============================================================================
// Criterion entry points
// ============================================================================

criterion_group!(
    benches,
    bench_init,
    bench_read_write,
    bench_snapshot,
    bench_ffi_serialize,
);
criterion_main!(benches);
