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

/// `laplace_sdk::rt::time` and `laplace_sdk::rt::laplace_select!` are the two
/// AXM2 A2-4 seams macro-generated code routes through — this proves both
/// are actually reachable via the `pub use laplace_rt as rt` facade (the
/// macro path is the interesting case: `#[macro_export]` hoists
/// `laplace_select!` to `laplace_rt`'s crate root, and this checks that
/// hoisted macro stays reachable through a re-exported module path, not just
/// `::laplace_rt::laplace_select!` directly).
#[tokio::test]
async fn public_facade_exports_rt_time_and_laplace_select_macro_path() {
    laplace_sdk::rt::time::sleep(std::time::Duration::from_millis(0)).await;

    let branch = laplace_sdk::rt::laplace_select! {
        v = async { 1_u8 } => v,
        v = async { 2_u8 } => v,
    };
    assert!(branch == 1 || branch == 2);
}
