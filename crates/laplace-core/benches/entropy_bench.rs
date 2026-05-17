//! Benchmarks for `laplace_core::domain::entropy`
//!
//! Covers three benchmark groups:
//! - `entropy/derive`     — seed derivation and RNG construction cost
//! - `entropy/throughput` — raw random number generation throughput
//! - `entropy/dpor`       — DPOR snapshot capture/restore latency

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use laplace_core::domain::entropy::{
    ContextId, DeterministicRng, Entropy, LocalSeed, SeedDerive, SystemEntropy,
};
use std::time::Duration;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// A. entropy/derive — seed derivation and RNG construction
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_derive(c: &mut Criterion) {
    let mut group = c.benchmark_group("entropy/derive");
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(3));

    let global_seed = 0xDEAD_BEEF_CAFE_BABEu64;
    let ctx_id = ContextId::new(42);

    // LocalSeed::derive: wrapping-arithmetic seed derivation formula
    group.bench_function("LocalSeed::derive", |b| {
        b.iter(|| {
            std::hint::black_box(LocalSeed::derive(
                std::hint::black_box(global_seed),
                std::hint::black_box(ctx_id),
            ))
        })
    });

    // DeterministicRng::new: ChaCha8 key schedule initialisation
    let seed = LocalSeed::derive(global_seed, ctx_id);
    group.bench_function("DeterministicRng::new", |b| {
        b.iter(|| {
            std::hint::black_box(DeterministicRng::new(
                std::hint::black_box(ctx_id),
                std::hint::black_box(seed),
            ))
        })
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// B. entropy/throughput — per-call generation cost
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("entropy/throughput");
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(3));
    group.throughput(Throughput::Elements(1));

    // SystemEntropy: OS-backed cryptographic randomness (OsRng)
    let sys = SystemEntropy::new();
    group.bench_function("SystemEntropy::next_u64", |b| {
        b.iter(|| std::hint::black_box(sys.next_u64()))
    });

    // DeterministicRng::next_u64: raw ChaCha8 block output
    let ctx_id = ContextId::new(1);
    let seed = LocalSeed::derive(12345, ctx_id);
    let mut rng = DeterministicRng::new(ctx_id, seed);
    group.bench_function("DeterministicRng::next_u64", |b| {
        b.iter(|| std::hint::black_box(rng.next_u64()))
    });

    // DeterministicRng::next_range: unbiased rejection-sampling range generation
    let mut rng_range = DeterministicRng::new(ctx_id, seed);
    group.bench_function("DeterministicRng::next_range(100)", |b| {
        b.iter(|| std::hint::black_box(rng_range.next_range(100)))
    });

    group.finish();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// C. entropy/dpor — DPOR time-machine snapshot cost
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn bench_dpor(c: &mut Criterion) {
    let mut group = c.benchmark_group("entropy/dpor");
    group.warm_up_time(Duration::from_millis(500));
    group.measurement_time(Duration::from_secs(3));

    let ctx_id = ContextId::new(7);
    let seed = LocalSeed::derive(98765, ctx_id);

    // Advance past the initial state to measure a realistic mid-stream snapshot.
    let mut rng = DeterministicRng::new(ctx_id, seed);
    for _ in 0..64 {
        let _ = rng.next_u64();
    }

    // capture_snapshot: clone the internal ChaCha8 stream position.
    // Uses &self so a single immutable borrow suffices.
    group.bench_function("capture_snapshot", |b| {
        b.iter(|| std::hint::black_box(rng.capture_snapshot()))
    });

    // restore_snapshot: overwrite the RNG state from a captured snapshot.
    //
    // setup and routine borrow `rng` differently (&self vs &mut self), so we
    // use two separate instances at the same stream offset to avoid the borrow
    // conflict while keeping the measurement representative.
    //
    // BatchSize::SmallInput ensures a fresh snapshot is prepared for each
    // sample without including capture cost in the measured restore latency.
    let rng_src = rng.clone(); // immutable source for setup
    let mut rng_dst = rng.clone(); // mutable target for routine
    group.bench_function("restore_snapshot", |b| {
        b.iter_batched(
            || rng_src.capture_snapshot(),
            |snapshot| rng_dst.restore_snapshot(std::hint::black_box(snapshot)),
            BatchSize::SmallInput,
        )
    });

    group.finish();
}

criterion_group!(entropy_benches, bench_derive, bench_throughput, bench_dpor);
criterion_main!(entropy_benches);
