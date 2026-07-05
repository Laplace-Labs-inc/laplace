// SPDX-License-Identifier: Apache-2.0

#[laplace_macro::laplace_tracked]
struct Gate {
    #[track]
    limiter: tokio::sync::Semaphore,
}

fn main() {}
