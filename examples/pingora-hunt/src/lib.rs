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

/// S7 N1′ — two reuse cycles with a customer-surface dynamic watcher.
///
/// Real-path correspondence: `pingora-core/src/connectors/mod.rs:271-278`
/// performs `put` and discards the native spawn handle, while
/// `pingora-pool/src/connection.rs:239-252` performs the pickup and
/// `:333-350` contains the terminal `idle_timeout` select. The Route B
/// counterpart is `s5_n1_reuse_cycle` in
/// `crates/laplace-cli/tests/pingora_hunt_engine.rs`.
///
/// The two `TaskHandle`s are awaited here only to keep both detached native
/// tasks inside this one-shot capture before the native TaskSet runner closes;
/// this is an adapter-only synchronization and is the documented deviation
/// from pingora's discarded-handle release path. Both timeout branches remain
/// disabled with `None`; pickup is the terminal signal for each cycle.
#[allow(dead_code)]
#[laplace_sdk::verify(tasks, name = "pingora_pool_s7_n1_reuse_cycle")]
fn pingora_pool_s7_n1(tasks: &mut laplace_sdk::rt::TaskSet) {
    let pool = Arc::new(ConnectionPool::new(2));
    let meta = ConnectionMeta::new(61, 1);
    let (notify_evicted, watch_use) = pool.put(&meta, "s7-n1-cycle-1".to_owned());
    let (first_closed_tx, notify_closed) = laplace_sdk::rt::watch::channel(false);

    let driver_pool = Arc::clone(&pool);
    tasks.spawn(async move {
        let _first_closed_tx = first_closed_tx;
        let first_pool = Arc::clone(&driver_pool);
        let first_meta = meta.clone();
        let first_watcher = tokio::spawn(async move {
            first_pool
                .idle_timeout(&first_meta, None, notify_evicted, notify_closed, watch_use)
                .await;
        });
        let _ = driver_pool.get(&meta.key);
        first_watcher
            .await
            .expect("S7 N1 first dynamic watcher must finish");

        let (notify_evicted, watch_use) = driver_pool.put(&meta, "s7-n1-cycle-2".to_owned());
        let (second_closed_tx, notify_closed) = laplace_sdk::rt::watch::channel(false);
        let second_pool = Arc::clone(&driver_pool);
        let second_meta = meta.clone();
        let second_watcher = tokio::spawn(async move {
            let _second_closed_tx = second_closed_tx;
            second_pool
                .idle_timeout(&second_meta, None, notify_evicted, notify_closed, watch_use)
                .await;
        });
        let _ = driver_pool.get(&meta.key);
        second_watcher
            .await
            .expect("S7 N1 second dynamic watcher must finish");
    });
}

/// S7 N2′ — eviction notification versus pickup on the customer surface.
///
/// Real-path correspondence: `pingora-core/src/connectors/mod.rs:271-278`
/// is the `put`/dynamic watcher release path, while
/// `pingora-pool/src/connection.rs:225-237,239-252` contains replacement
/// eviction and competing pickup. The watcher terminal select is
/// `pingora-pool/src/connection.rs:333-350`; the Route B counterpart is
/// `s5_n2_eviction_vs_pickup` in
/// `crates/laplace-cli/tests/pingora_hunt_engine.rs`.
///
/// The watcher is a statement-position spawn to mirror the discarded native
/// handle. Capacity overflow or pickup is the only intended termination; the
/// timeout branch is disabled with `None`.
#[allow(dead_code)]
#[laplace_sdk::verify(tasks, name = "pingora_pool_s7_n2_eviction_vs_pickup")]
fn pingora_pool_s7_n2(tasks: &mut laplace_sdk::rt::TaskSet) {
    let pool = Arc::new(ConnectionPool::new(1));
    let first_meta = ConnectionMeta::new(62, 1);
    let (notify_evicted, watch_use) = pool.put(&first_meta, "s7-n2-first".to_owned());
    let (closed_tx, notify_closed) = laplace_sdk::rt::watch::channel(false);

    let watcher_pool = Arc::clone(&pool);
    let watcher_meta = first_meta.clone();
    tasks.spawn(async move {
        tokio::spawn(async move {
            let _closed_tx = closed_tx;
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
    });

    let evictor_pool = Arc::clone(&pool);
    tasks.spawn(async move {
        let second_meta = ConnectionMeta::new(62, 2);
        evictor_pool.put(&second_meta, "s7-n2-second".to_owned());
    });

    let getter_pool = Arc::clone(&pool);
    tasks.spawn(async move {
        let _ = getter_pool.get(&first_meta.key);
    });
}

/// S7 N3′ — close notification, including `pop_closed`, versus pickup.
///
/// Real-path correspondence: `pingora-core/src/connectors/mod.rs:271-278`
/// creates the idle watcher, `pingora-pool/src/connection.rs:247-249,321-330`
/// covers `get`/`pop_closed`, and `:333-350` is the terminal select. The
/// Route B counterpart is `s5_n3_close_notify_vs_pickup` in
/// `crates/laplace-cli/tests/pingora_hunt_engine.rs`.
///
/// The dynamic watcher uses statement-position spawn so its native handle is
/// discarded like the real release path. The close sender and pickup task are
/// the signal-terminating competitors; timeout remains disabled with `None`.
#[allow(dead_code)]
#[laplace_sdk::verify(tasks, name = "pingora_pool_s7_n3_close_notify_vs_pickup")]
fn pingora_pool_s7_n3(tasks: &mut laplace_sdk::rt::TaskSet) {
    let pool = Arc::new(ConnectionPool::new(2));
    let meta = ConnectionMeta::new(63, 1);
    let (notify_evicted, watch_use) = pool.put(&meta, "s7-n3-connection".to_owned());
    let (closed_tx, notify_closed) = laplace_sdk::rt::watch::channel(false);

    let watcher_pool = Arc::clone(&pool);
    let watcher_meta = meta.clone();
    let closer = closed_tx.clone();
    tasks.spawn(async move {
        tokio::spawn(async move {
            let _closed_tx = closed_tx;
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
    });

    tasks.spawn(async move {
        let _ = closer.send(true);
    });

    let getter_pool = Arc::clone(&pool);
    tasks.spawn(async move {
        let _ = getter_pool.get(&meta.key);
    });
}
