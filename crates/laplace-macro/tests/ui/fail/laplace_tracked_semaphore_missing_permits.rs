// SPDX-License-Identifier: MIT

#[laplace_macro::laplace_tracked]
struct Gate {
    #[track]
    limiter: tokio::sync::Semaphore,
}

fn main() {}
