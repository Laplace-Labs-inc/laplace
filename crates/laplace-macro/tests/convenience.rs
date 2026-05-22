// SPDX-License-Identifier: Apache-2.0
#![cfg(feature = "closed-sdk-tests")]

#[tokio::test]
async fn test_mutex_macro_creates_tracked_mutex() {
    let lock = laplace_macro::mutex!(41u64, "counter");
    let mut guard = lock.lock().await;
    *guard += 1;
    assert_eq!(*guard, 42);
}

#[tokio::test]
async fn test_rwlock_macro_creates_tracked_rwlock() {
    let lock = laplace_macro::rwlock!(String::from("alpha"), "name");
    {
        let mut guard = lock.write().await;
        guard.push_str("-2");
    }
    let guard = lock.read().await;
    assert_eq!(guard.as_str(), "alpha-2");
}
