// SPDX-License-Identifier: Apache-2.0
#![allow(unused_imports)]

#[laplace_macro::model]
fn uses_qualified_spawn() {
    let handle = std::thread::spawn(|| {});
    handle.join().expect("thread completes");
}

#[laplace_macro::model]
fn uses_absolute_spawn() {
    let handle = ::std::thread::spawn(|| {});
    handle.join().expect("thread completes");
}

#[laplace_macro::model]
fn uses_thread_module_spawn() {
    use std::thread;

    let handle = thread::spawn(|| {});
    handle.join().expect("thread completes");
}

fn main() {}
