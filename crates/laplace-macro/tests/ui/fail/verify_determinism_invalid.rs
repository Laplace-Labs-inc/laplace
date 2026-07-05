// SPDX-License-Identifier: Apache-2.0

#[laplace_macro::laplace_verify(threads = 1, determinism = "best_effort")]
async fn determinism_invalid() {}

fn main() {}
