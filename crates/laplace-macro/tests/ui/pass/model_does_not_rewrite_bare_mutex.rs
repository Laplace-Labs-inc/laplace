// SPDX-License-Identifier: Apache-2.0
#![allow(unused_imports)]

use std::sync::Mutex;

#[laplace_macro::model]
fn keeps_bare_mutex_unchanged() {
    let value = Mutex::new(1_u8);
    let guard = value.lock().expect("lock succeeds");
    assert_eq!(*guard, 1);
}

fn main() {
    keeps_bare_mutex_unchanged();
}
