// SPDX-License-Identifier: Apache-2.0

use laplace_sdk::prelude::*;

#[laplace_tracked]
struct Unsupported {
    #[track]
    values: Vec<u64>,
}

fn main() {}
