// SPDX-License-Identifier: MIT

#[laplace_macro::laplace_tracked]
struct Gate {
    #[track(permits = 2)]
    limiter: tokio::sync::Semaphore,
}

fn main() {
    let gate = Gate::default();
    let _limiter = gate.limiter;
}
