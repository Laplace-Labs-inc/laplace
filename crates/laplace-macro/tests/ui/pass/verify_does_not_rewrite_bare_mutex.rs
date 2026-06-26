// SPDX-License-Identifier: Apache-2.0
#![allow(unused_imports, dead_code)]

use std::sync::Mutex;

// Same invariant as the `#[laplace::model]` bare-Mutex test: an unqualified
// `Mutex` (e.g. re-exported / parking_lot coexistence) is NOT rewritten, since
// token-only expansion cannot prove it came from `std::sync::Mutex`.
#[laplace_macro::laplace_verify(threads = 2)]
async fn verify_keeps_bare_mutex_unchanged() {
    let value: Mutex<u8> = Mutex::new(1_u8);
    let guard = value.lock().expect("lock succeeds");
    assert_eq!(*guard, 1);
}

fn main() {}
