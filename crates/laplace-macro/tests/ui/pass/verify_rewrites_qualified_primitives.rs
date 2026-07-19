// SPDX-License-Identifier: Apache-2.0
#![allow(unused_imports, dead_code)]

use std::sync::Arc;

// Single-annotation control layer: `#[laplace::verify]` alone now performs the
// model rewrite that previously required a separate `#[laplace::model]` line.
// The explicit `::laplace_model_rt::ModelMutex` / `::laplace_model_rt::spawn` type bindings
// below only typecheck if the qualified `std` primitives were rewritten.
#[laplace_macro::laplace_verify(threads = 2)]
async fn verify_rewrites_qualified_mutex() {
    let value: Arc<laplace_model_rt::ModelMutex<u8>> = Arc::new(std::sync::Mutex::new(1_u8));
    let guard = value.lock().expect("lock succeeds");
    assert_eq!(*guard, 1);
}

#[laplace_macro::laplace_verify(threads = 2)]
async fn verify_rewrites_absolute_mutex() {
    let value: ::std::sync::Arc<::std::sync::Mutex<u8>> =
        ::std::sync::Arc::new(::std::sync::Mutex::new(2_u8));
    let guard = value.lock().expect("lock succeeds");
    assert_eq!(*guard, 2);
}

#[laplace_macro::laplace_verify(threads = 2)]
async fn verify_rewrites_qualified_spawn() {
    let handle = std::thread::spawn(|| {});
    handle.join().expect("thread completes");
}

fn main() {}
