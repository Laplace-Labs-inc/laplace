// SPDX-License-Identifier: MIT
#![allow(dead_code)]

#[derive(Default)]
struct State {
    value: usize,
}

#[laplace_macro::laplace_verify(threads = 1)]
fn sync_ref(state: &State) {
    assert_eq!(state.value, 0);
}

fn main() {}
