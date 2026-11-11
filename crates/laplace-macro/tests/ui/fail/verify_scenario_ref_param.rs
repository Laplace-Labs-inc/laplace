// SPDX-License-Identifier: MIT

#[derive(Default)]
struct State;

#[laplace_macro::laplace_verify(scenario)]
fn scenario_ref_param(_state: &State) {}

fn main() {}
