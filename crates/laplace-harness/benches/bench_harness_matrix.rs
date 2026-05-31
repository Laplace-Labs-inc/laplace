// SPDX-License-Identifier: Apache-2.0
//! Local deterministic microbenchmarks for the SSOT harness performance matrix.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::collections::BTreeMap;
use std::time::Duration;

const DEFAULT_SEED: u64 = 42;

#[derive(Clone)]
struct Fixture {
    name: String,
    threads: usize,
    resources: usize,
    max_depth: usize,
}

fn bench_seed() -> u64 {
    std::env::var("LAPLACE_BENCH_SEED")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_SEED)
}

fn setup_harness(seed: u64) -> Vec<Fixture> {
    (0..64)
        .map(|index| Fixture {
            name: format!("fixture_{seed}_{index}"),
            threads: (index % 4) + 1,
            resources: (index % 8) + 1,
            max_depth: 32 + index,
        })
        .collect()
}

fn install_fixtures(seed: u64) -> usize {
    let fixtures = setup_harness(seed);
    let mut registry = BTreeMap::new();

    for fixture in fixtures {
        let weight = fixture.threads * fixture.resources * fixture.max_depth;
        registry.insert(fixture.name, weight);
    }

    registry
        .values()
        .fold(0usize, |acc, weight| acc.wrapping_add(*weight))
}

fn harness_determinism_digest(seed: u64) -> u64 {
    setup_harness(seed).into_iter().fold(0u64, |acc, fixture| {
        let name_digest = fixture.name.bytes().fold(0u64, |name_acc, byte| {
            name_acc.wrapping_add(u64::from(byte))
        });
        acc.wrapping_add(name_digest)
            .wrapping_add(fixture.threads as u64)
            .wrapping_add(fixture.resources as u64)
            .wrapping_add(fixture.max_depth as u64)
    })
}

fn bench_harness_setup(c: &mut Criterion) {
    let seed = bench_seed();
    c.bench_function("harness_setup_ns", |b| {
        b.iter(|| {
            let fixtures = setup_harness(black_box(seed));
            black_box(fixtures)
        })
    });
}

fn bench_fixture_install(c: &mut Criterion) {
    let seed = bench_seed();
    c.bench_function("harness_fixture_install_ns", |b| {
        b.iter(|| {
            let total_weight = install_fixtures(black_box(seed));
            black_box(total_weight)
        })
    });
}

fn bench_harness_determinism_replay(c: &mut Criterion) {
    c.bench_function("harness_determinism_replay_3runs", |b| {
        b.iter(|| {
            let r1 = harness_determinism_digest(black_box(DEFAULT_SEED));
            let r2 = harness_determinism_digest(black_box(DEFAULT_SEED));
            let r3 = harness_determinism_digest(black_box(DEFAULT_SEED));
            assert_eq!(r1, r2);
            assert_eq!(r2, r3);
            black_box((r1, r2, r3))
        })
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(100)
        .warm_up_time(Duration::from_secs(5))
        .measurement_time(Duration::from_secs(10))
        .confidence_level(0.95)
        .significance_level(0.05);
    targets = bench_harness_setup, bench_fixture_install, bench_harness_determinism_replay
}
criterion_main!(benches);
