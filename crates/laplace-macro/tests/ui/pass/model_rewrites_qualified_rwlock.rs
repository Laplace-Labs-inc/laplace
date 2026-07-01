// SPDX-License-Identifier: Apache-2.0
#![allow(unused_imports)]

use std::sync::Arc;

#[laplace_macro::model]
fn uses_qualified_std_rwlock() {
    // Type ascription proves the rewrite std::sync::RwLock -> laplace_rt::ModelRwLock.
    let value: Arc<laplace_rt::ModelRwLock<u8>> = Arc::new(std::sync::RwLock::new(1_u8));
    {
        let r = value.read().expect("read succeeds");
        assert_eq!(*r, 1);
    }
    {
        let mut w = value.write().expect("write succeeds");
        *w = 2;
    }
    assert_eq!(*value.read().expect("read succeeds"), 2);
}

#[laplace_macro::model]
fn uses_try_lock_on_std_mutex() {
    let value: laplace_rt::ModelMutex<u8> = std::sync::Mutex::new(3);
    let guard = value.try_lock().expect("uncontended try_lock succeeds");
    assert_eq!(*guard, 3);
}

fn main() {
    uses_qualified_std_rwlock();
    uses_try_lock_on_std_mutex();
}
