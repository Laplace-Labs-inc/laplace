// SPDX-License-Identifier: Apache-2.0

#[laplace_sdk::verify(threads = 1, expected = "clean")]
async fn no_state_smoke() {}

fn main() {}
