// SPDX-License-Identifier: MIT

#[laplace_macro::laplace_verify(threads = 1, expected = "maybe")]
async fn expected_invalid() {}

fn main() {}
