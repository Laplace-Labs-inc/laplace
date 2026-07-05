// SPDX-License-Identifier: Apache-2.0

#[derive(Default)]
struct State;

#[laplace_macro::laplace_verify(threads = 1)]
async fn value_param(_state: State) {}

fn main() {}
