// SPDX-License-Identifier: Apache-2.0
#![allow(dead_code)]

#[laplace_macro::laplace_verify(threads = 1)]
fn sync_no_args() {
    let value = 1 + 1;
    assert_eq!(value, 2);
}

fn main() {}
