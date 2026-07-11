//! Route A: native capture of pingora-pool's idle-timeout protocol.

use std::sync::Arc;

use pingora_pool_patched::{ConnectionMeta, ConnectionPool};

/// S1 — owner pickup races the idle watcher, with no wall-clock timeout.
///
/// The closed sender is held until the watcher completes so the selected
/// protocol signal is the pool's `get()` -> oneshot release, not watch closure.
#[allow(dead_code)]
#[laplace_sdk::verify(tasks, name = "pingora_pool_s1_pickup_vs_timeout")]
fn pingora_pool_s1(tasks: &mut laplace_sdk::rt::TaskSet) {
    let pool = Arc::new(ConnectionPool::new(2));
    let meta = ConnectionMeta::new(7, 1);
    let (notify_evicted, watch_use) = pool.put(&meta, "s1-connection".to_owned());
    let (closed_tx, notify_closed) = laplace_sdk::rt::watch::channel(false);

    let watcher_pool = Arc::clone(&pool);
    let watcher_meta = meta.clone();
    let watcher = tasks.spawn(async move {
        watcher_pool
            .idle_timeout(
                &watcher_meta,
                None,
                notify_evicted,
                notify_closed,
                watch_use,
            )
            .await;
    });

    let owner_pool = Arc::clone(&pool);
    tasks.spawn(async move {
        let _closed_tx = closed_tx;
        let _ = owner_pool.get(&meta.key);
        watcher.await;
    });
}
