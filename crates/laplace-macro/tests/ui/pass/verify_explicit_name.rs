// SPDX-License-Identifier: MIT
#![allow(dead_code)]

#[laplace_macro::laplace_verify(threads = 1, name = "explicit_target", expected = "clean")]
async fn explicit_name() {}

fn main() {}
