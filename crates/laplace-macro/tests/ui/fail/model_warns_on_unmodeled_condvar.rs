// SPDX-License-Identifier: Apache-2.0
//
// `#[laplace::model]` must never let an un-modeled concurrency primitive pass
// silently (anti-false-green, day-1 non-negotiable). `Condvar` is recognized
// but not modeled, so the rewrite injects a deprecated marker whose note names
// the blind spot. `#![deny(deprecated)]` promotes that honest warning to an
// error, proving the mechanism fires.
#![deny(deprecated)]
#![allow(unused_variables)]

use std::sync::{Condvar, Mutex};

#[laplace_macro::model]
fn waits_on_unmodeled_condvar() {
    let lock = Mutex::new(false);
    let cvar = Condvar::new();
    let guard = lock.lock().expect("lock succeeds");
    let _ = &cvar;
}

fn main() {
    waits_on_unmodeled_condvar();
}
