// SPDX-License-Identifier: Apache-2.0

#[laplace_macro::laplace_tracked]
struct Service<T: Default> {
    #[track]
    lock: tokio::sync::Mutex<u64>,
    extra: T,
}

fn main() {
    let service = Service::<String>::default();
    let _lock = service.lock;
    assert_eq!(service.extra, String::default());
}
