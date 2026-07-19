// SPDX-License-Identifier: Apache-2.0

#[laplace_macro::laplace_verify(tasks, threads = 2)]
fn composition_with_two_modes(_tasks: &mut laplace_model_rt::TaskSet) {}

fn main() {}
