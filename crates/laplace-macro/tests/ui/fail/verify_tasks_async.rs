// SPDX-License-Identifier: Apache-2.0

#[laplace_macro::laplace_verify(tasks)]
async fn async_composition(_tasks: &mut laplace_model_rt::TaskSet) {}

fn main() {}
