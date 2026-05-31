// SPDX-License-Identifier: Apache-2.0
//! SSOT-aligned local benchmark matrix for `laplace-interfaces`.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::time::Duration;

const SEED: u64 = 42;
const DISPATCH_INPUTS: usize = 1_024;

trait InterfaceDispatch {
    fn route(&self, value: u64) -> u64;
}

#[derive(Clone, Copy)]
struct StaticInterface {
    salt: u64,
}

impl InterfaceDispatch for StaticInterface {
    fn route(&self, value: u64) -> u64 {
        value.rotate_left(13) ^ self.salt
    }
}

extern "C" fn abi_roundtrip(value: u64) -> u64 {
    value.rotate_left(7) ^ SEED
}

fn assert_seed() {
    let seed = std::env::var("LAPLACE_BENCH_SEED")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(SEED);
    assert_eq!(
        seed, SEED,
        "laplace-interfaces benchmarks require LAPLACE_BENCH_SEED=42"
    );
}

fn static_dispatch<T: InterfaceDispatch>(interface: T, value: u64) -> u64 {
    interface.route(value)
}

fn seeded_inputs(seed: u64, len: usize) -> Vec<u64> {
    let mut state = seed;
    (0..len)
        .map(|index| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            state ^ (index as u64).rotate_left((index % 17) as u32)
        })
        .collect()
}

fn static_dispatch_digest(interface: StaticInterface, inputs: &[u64]) -> u64 {
    inputs.iter().fold(0u64, |acc, value| {
        let routed = static_dispatch(black_box(interface), black_box(*value));
        acc.wrapping_add(routed.rotate_left((value & 31) as u32))
    })
}

fn dyn_dispatch_digest(interface: &dyn InterfaceDispatch, inputs: &[u64]) -> u64 {
    inputs.iter().fold(0u64, |acc, value| {
        let routed = black_box(interface).route(black_box(*value));
        acc.wrapping_add(routed.rotate_left((value & 31) as u32))
    })
}

fn abi_call_digest(inputs: &[u64]) -> u64 {
    inputs.iter().fold(0u64, |acc, value| {
        let routed = abi_roundtrip(black_box(*value));
        acc.wrapping_add(routed.rotate_left((value & 31) as u32))
    })
}

fn deterministic_replay_digest(seed: u64, inputs: &[u64]) -> u64 {
    let interface = StaticInterface { salt: seed };
    let dyn_interface: &dyn InterfaceDispatch = &interface;

    static_dispatch_digest(interface, inputs)
        ^ dyn_dispatch_digest(dyn_interface, inputs).rotate_left(7)
        ^ abi_call_digest(inputs).rotate_left(13)
}

fn interfaces_trait_dispatch(c: &mut Criterion) {
    assert_seed();
    let interface = StaticInterface { salt: SEED };
    let dyn_interface: &dyn InterfaceDispatch = &interface;
    let inputs = seeded_inputs(SEED, DISPATCH_INPUTS);
    let mut group = c.benchmark_group("interfaces/trait_dispatch_overhead_dyn_vs_static");

    group.bench_function("interfaces_trait_dispatch_overhead_static", |b| {
        b.iter(|| {
            let digest = static_dispatch_digest(black_box(interface), black_box(&inputs));
            black_box(digest)
        })
    });

    group.bench_function("interfaces_trait_dispatch_overhead_dyn", |b| {
        b.iter(|| {
            let digest = dyn_dispatch_digest(black_box(dyn_interface), black_box(&inputs));
            black_box(digest)
        })
    });

    group.finish();
}

fn interfaces_ffi_abi_call(c: &mut Criterion) {
    assert_seed();
    let inputs = seeded_inputs(SEED, DISPATCH_INPUTS);
    c.bench_function("interfaces_ffi_abi_call_cost", |b| {
        b.iter(|| {
            let digest = abi_call_digest(black_box(&inputs));
            black_box(digest)
        })
    });
}

fn interfaces_determinism_replay(c: &mut Criterion) {
    assert_seed();
    let inputs = seeded_inputs(SEED, 16);
    c.bench_function("interfaces_determinism_replay_3runs", |b| {
        b.iter(|| {
            let r1 = deterministic_replay_digest(black_box(SEED), black_box(&inputs));
            let r2 = deterministic_replay_digest(black_box(SEED), black_box(&inputs));
            let r3 = deterministic_replay_digest(black_box(SEED), black_box(&inputs));
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
    targets =
        interfaces_trait_dispatch,
        interfaces_ffi_abi_call,
        interfaces_determinism_replay
}
criterion_main!(benches);
