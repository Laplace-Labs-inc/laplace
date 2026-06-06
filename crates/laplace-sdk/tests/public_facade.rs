// SPDX-License-Identifier: Apache-2.0

use laplace_sdk::prelude::*;

#[tokio::test]
async fn public_facade_exports_tracked_primitives() {
    let lock = TrackedMutex::named(1usize, "facade_mutex");
    let mut guard = lock.lock().await;
    *guard += 1;

    assert_eq!(*guard, 2);
}

#[test]
fn public_facade_exports_project_config_loader() {
    let _config = laplace_sdk::load_project_config();
}
