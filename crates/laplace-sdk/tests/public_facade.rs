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
/// are actually reachable via the `pub use laplace_model_rt as rt` facade (the
/// macro path is the interesting case: `#[macro_export]` hoists
/// `laplace_select!` to `laplace_model_rt`'s crate root, and this checks that
/// hoisted macro stays reachable through a re-exported module path, not just
/// `::laplace_model_rt::laplace_select!` directly).
#[tokio::test]
async fn public_facade_exports_rt_time_and_laplace_select_macro_path() {
    laplace_sdk::rt::time::sleep(std::time::Duration::from_millis(0)).await;

    let branch = laplace_sdk::rt::laplace_select! {
        v = async { 1_u8 } => v,
        v = async { 2_u8 } => v,
    };
    assert!(branch == 1 || branch == 2);
}

/// AXM2 A2-5 runtime-level proof: the *dominant* crates.io style —
/// `use tokio::sync::mpsc;` followed by `mpsc::channel(1)`, inside an
/// inline `mod` annotated with `#[laplace_sdk::model]` instead of a bare
/// `fn` — actually compiles against and runs through the
/// `laplace_sdk::rt::mpsc` shadow channel, not real `tokio::sync::mpsc`.
/// Unlike the macro crate's trybuild pass tests (compile-only), this
/// executes the round trip end to end.
#[laplace_sdk::model]
mod aliased_mpsc_style {
    // The rewrite replaces every `mpsc::...` call site below with a fully
    // qualified `::laplace_sdk::rt::mpsc::...` path, so this import is no
    // longer referenced in the *expanded* code — it is what makes the
    // alias resolvable in the first place.
    #[allow(unused_imports)]
    use tokio::sync::mpsc;

    pub async fn round_trip(value: u8) -> u8 {
        let (tx, mut rx) = mpsc::channel::<u8>(1);
        tx.send(value).await.expect("send succeeds");
        rx.recv().await.expect("recv succeeds")
    }
}

#[tokio::test]
async fn public_facade_rewrites_aliased_tokio_mpsc_inside_annotated_module() {
    let value = aliased_mpsc_style::round_trip(7).await;
    assert_eq!(value, 7);
}
