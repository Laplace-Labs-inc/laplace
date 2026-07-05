// SPDX-License-Identifier: Apache-2.0
// Pins the `max_depth` emission contract: `ProbeSessionConfig.max_depth` is a
// plain `usize`, so the macro must emit `max_depth: N` (not `Some(N)`).
#![allow(dead_code)]

#[derive(Default)]
struct State {
    value: usize,
}

#[laplace_macro::laplace_verify(threads = 1, max_depth = 64)]
fn bounded_depth(state: &State) {
    assert_eq!(state.value, 0);
}

fn main() {}
