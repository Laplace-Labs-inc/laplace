// SPDX-License-Identifier: MIT

use laplace_sdk::prelude::*;

#[laplace_tracked]
struct Unsupported {
    #[track]
    values: Vec<u64>,
}

fn main() {}
