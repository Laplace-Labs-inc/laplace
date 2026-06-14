// SPDX-License-Identifier: Apache-2.0
#![allow(unused_imports)]

use std::sync::Arc;

#[laplace_macro::model]
fn uses_qualified_std_mutex() {
    let value: Arc<laplace_rt::ModelMutex<u8>> = Arc::new(std::sync::Mutex::new(1_u8));
    let guard = value.lock().expect("lock succeeds");
    assert_eq!(*guard, 1);
}

#[laplace_macro::model]
fn uses_absolute_std_mutex() {
    let value: ::std::sync::Arc<::std::sync::Mutex<u8>> =
        ::std::sync::Arc::new(::std::sync::Mutex::new(2_u8));
    let guard = value.lock().expect("lock succeeds");
    assert_eq!(*guard, 2);
}

#[laplace_macro::model]
fn uses_turbofish_std_mutex_new() {
    let value: laplace_rt::ModelMutex<u8> = std::sync::Mutex::<u8>::new(3);
    let guard = value.lock().expect("lock succeeds");
    assert_eq!(*guard, 3);
}

fn main() {
    uses_qualified_std_mutex();
    uses_absolute_std_mutex();
    uses_turbofish_std_mutex_new();
}
