// SPDX-License-Identifier: Apache-2.0
#![allow(dead_code)]

#[laplace_macro::laplace_verify(threads = 1, name = "explicit_target", expected = "clean")]
async fn explicit_name() {}

fn main() {}
