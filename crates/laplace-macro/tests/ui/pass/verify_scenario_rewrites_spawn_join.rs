// SPDX-License-Identifier: Apache-2.0
#![allow(dead_code)]

use std::sync::Arc;

#[laplace_macro::laplace_verify(scenario, expected = "clean")]
fn scenario_rewrites_spawn_join() {
    let value: Arc<laplace_rt::ModelMutex<u8>> = Arc::new(std::sync::Mutex::new(1_u8));
    let worker_value = Arc::clone(&value);
    let handle = std::thread::spawn(move || {
        let guard = worker_value.lock().expect("lock succeeds");
        assert_eq!(*guard, 1);
    });

    handle.join().unwrap();
}

fn main() {}
