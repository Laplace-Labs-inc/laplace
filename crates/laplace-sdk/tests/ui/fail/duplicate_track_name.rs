// SPDX-License-Identifier: Apache-2.0

use laplace_sdk::prelude::*;

#[laplace_tracked]
struct DuplicateNames {
    #[track(name = "shared")]
    left: Mutex<u64>,

    #[track(name = "shared")]
    right: Mutex<u64>,
}

fn main() {}
