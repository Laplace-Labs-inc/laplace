// SPDX-License-Identifier: Apache-2.0

#[laplace_macro::laplace_verify(threads = 1, threds = 2)]
async fn unknown_arg() {}

fn main() {}
