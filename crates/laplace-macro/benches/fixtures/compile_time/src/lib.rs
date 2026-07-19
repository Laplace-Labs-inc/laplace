// SPDX-License-Identifier: Apache-2.0
#![allow(dead_code)]

use laplace_macro::laplace_meta;

#[laplace_meta(bench = "compile_time", seed = 42)]
pub fn passthrough_attribute_fixture() -> u64 {
    42
}
