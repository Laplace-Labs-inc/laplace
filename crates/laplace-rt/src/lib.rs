// SPDX-License-Identifier: Apache-2.0
#![deny(clippy::all, clippy::pedantic)]

//! Runtime seams for annotated Laplace model code.
//!
//! `spawn` routes unit-returning model threads through an installed engine hook;
//! `spawn_task` routes fire-and-forget async futures through an installed async
//! engine hook and delegates to `tokio::spawn` without one.
//! `ModelMutex` routes qualified `std::sync::Mutex` annotated by
//! `#[laplace::model]` through an installed lock hook. With no hook installed,
//! these surfaces delegate to the standard library.
//!
//! P-1 only supports `FnOnce() -> ()` closures. Non-unit spawn returns,
//! `std::thread::Builder`, scoped threads, and unqualified bare `spawn(...)`
//! calls are outside this runtime surface. The separate `TaskSet` surface
//! provides pre-registered native async task composition.
//!
//! P-3 rewrites qualified `std::sync::Mutex` and `std::sync::RwLock` paths and
//! supports `try_lock`/`try_read`/`try_write` (a non-blocking acquisition that
//! succeeds reports one acquire boundary; a `WouldBlock` failure reports
//! nothing). `RwLock` read and write both route through the same *exclusive*
//! boundary — sound for the deadlock/liveness oracle (readers are serialized,
//! never producing a false wait cycle) but concurrent-reader interleavings are
//! not explored. Bare `Mutex`/`RwLock`, `sync::Mutex`, poison recovery helpers,
//! `Condvar::wait`, mapped guards, and scoped threads remain outside this
//! surface; `Condvar`, `std::sync::atomic`, and `mpsc` channels encountered in
//! annotated source are flagged as un-modeled blind spots via
//! [`unmodeled`] rather than silently passing.
//!
//! AXM2 A2-3 adds the async side of this seam: [`async_mutex`],
//! [`async_rwlock`], and [`async_semaphore`] rewrite qualified
//! `tokio::sync::{Mutex,RwLock,Semaphore}` to wrap-real model types, and
//! [`async_notify`] provides a `tokio::sync::Notify`-compatible model type —
//! see each module's honesty contract for what is and is not observable
//! through it. AXM2 A2-4 adds the `mpsc`/oneshot/watch channel family:
//! [`mpsc`], [`oneshot`], and [`watch`] rewrite qualified
//! `tokio::sync::{mpsc,oneshot,watch}` constructors and types to wrap-real
//! model channels, and BCAST G4 keep (LEP-0027) completes the family with
//! [`broadcast`] — all four `tokio::sync` channel primitives are modeled.
//! `std::sync::mpsc` remains flagged via [`unmodeled`]; `tokio::spawn` is
//! also flagged as un-modeled (the deterministic executor does not yet
//! control it).
//!
//! AXM2 A2-4 adds [`time`] — a `tokio::time::{sleep,timeout,interval}`
//! virtual-clock shadow driven by [`AsyncTimerHook`] — and
//! [`laplace_select`], a runtime-gated `tokio::select!` drop-in (see the
//! macro's own doc for the gate).
//!
//! ## Module layout
//!
//! - [`hooks`] — engine hook traits + install/clear + resource-id allocation.
//! - [`spawn`] — model-thread and fire-and-forget async spawn seams.
//! - [`mutex`] — [`ModelMutex`].
//! - [`rwlock`] — [`ModelRwLock`].
//! - [`async_mutex`] — [`ModelAsyncMutex`], the `tokio::sync::Mutex` seam.
//! - [`async_rwlock`] — [`ModelAsyncRwLock`], the `tokio::sync::RwLock` seam.
//! - [`async_semaphore`] — [`ModelAsyncSemaphore`], the `tokio::sync::Semaphore` seam.
//! - [`async_notify`] — [`ModelAsyncNotify`], the `tokio::sync::Notify` seam.
//! - [`mpsc`] — the `tokio::sync::mpsc` (bounded + unbounded) seam.
//! - [`oneshot`] — the `tokio::sync::oneshot` seam.
//! - [`watch`] — the `tokio::sync::watch` seam.
//! - [`async_time`] — [`time`], the `tokio::time` virtual-clock seam.
//! - [`select_macro`] — [`laplace_select`], the `tokio::select!` seam.
//! - [`unmodeled`] — compile-time blind-spot markers.

#[cfg(feature = "arc-swap")]
pub mod arc_swap;
mod async_mutex;
mod async_notify;
mod async_rwlock;
mod async_semaphore;
mod async_time;
pub mod broadcast;
mod hooks;
pub mod mpsc;
mod mutex;
pub mod oneshot;
mod rwlock;
mod select_macro;
mod spawn;
mod task_set;
pub mod unmodeled;
pub mod watch;

#[cfg(feature = "arc-swap")]
pub use arc_swap::{
    Cache, ModelArcSwap, ModelArcSwapGuard, ModelArcSwapOption, ModelArcSwapOptionGuard,
};
pub use async_mutex::{ModelAsyncLock, ModelAsyncMutex, ModelAsyncMutexGuard};
pub use async_notify::{ModelAsyncNotify, ModelNotified};
pub use async_rwlock::{
    ModelAsyncRead, ModelAsyncRwLock, ModelAsyncRwLockReadGuard, ModelAsyncRwLockWriteGuard,
    ModelAsyncWrite,
};
pub use async_semaphore::{ModelAsyncSemaphore, ModelSemaphoreAcquire, ModelSemaphorePermit};
pub use hooks::{
    clear_async_broadcast_hook, clear_async_cell_hook, clear_async_channel_hook,
    clear_async_lock_hook, clear_async_notify_hook, clear_async_spawn_hook, clear_async_timer_hook,
    clear_lock_hook, clear_spawn_hook, clear_task_observer_hook, deterministic_select_enabled,
    install_async_broadcast_hook, install_async_cell_hook, install_async_channel_hook,
    install_async_lock_hook, install_async_notify_hook, install_async_spawn_hook,
    install_async_timer_hook, install_lock_hook, install_spawn_hook, install_task_observer_hook,
    reset_model_async_ids_for_model, reset_model_mutex_ids_for_model, set_deterministic_select,
    AsyncAcquireKind, AsyncBroadcastHook, AsyncBroadcastOp, AsyncBroadcastOutcome, AsyncCellHook,
    AsyncChannelHook, AsyncChannelKind, AsyncChannelOp, AsyncChannelOutcome, AsyncChannelSide,
    AsyncLockHook, AsyncNotifyHook, AsyncSpawnHook, AsyncTimerHook, LockHook, SpawnHook,
    TaskObserverHook, TaskPollOutcome,
};
pub use mutex::{ModelMutex, ModelMutexGuard};
pub use rwlock::{ModelRwLock, ModelRwLockReadGuard, ModelRwLockWriteGuard};
pub use spawn::{
    spawn, spawn_task, JoinToken, TaskControl, TaskControlState, TaskHandle, TaskJoinError,
};
pub use task_set::{TaskHandle as TaskSetHandle, TaskSet};

/// `tokio::time`-compatible model virtual-clock shadow (`sleep`, `timeout`,
/// `interval`). See [`async_time`] for the honesty contract.
pub mod time {
    pub use crate::async_time::{interval, sleep, timeout, Elapsed, Interval, Sleep, Timeout};
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError, TryLockError};
    use std::time::Duration;

    /// Serializes every test that touches the process-global lock hook and
    /// resource-id counter, so `cargo test`'s default parallelism cannot let one
    /// test's `reset`/`install`/`clear` stomp another's expectations.
    static TEST_GUARD: Mutex<()> = Mutex::new(());

    /// Acquires the serialization guard, recovering from a poisoned guard left
    /// by an unrelated panicking test.
    fn serial() -> MutexGuard<'static, ()> {
        TEST_GUARD.lock().unwrap_or_else(PoisonError::into_inner)
    }

    struct RecordingLockHook {
        events: Mutex<Vec<(&'static str, u64)>>,
    }

    impl RecordingLockHook {
        const fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
            }
        }

        fn events(&self) -> Vec<(&'static str, u64)> {
            self.events.lock().expect("events lock").clone()
        }
    }

    impl LockHook for RecordingLockHook {
        fn acquire(&self, resource: u64) {
            self.events
                .lock()
                .expect("events lock")
                .push(("acquire", resource));
        }

        fn release(&self, resource: u64) {
            self.events
                .lock()
                .expect("events lock")
                .push(("release", resource));
        }
    }

    #[test]
    fn model_mutex_routes_acquire_and_release_to_installed_hook() {
        let _serial = serial();
        reset_model_mutex_ids_for_model();
        clear_lock_hook();
        clear_spawn_hook();
        let hook = Arc::new(RecordingLockHook::new());
        install_lock_hook(hook.clone());

        let mutex = ModelMutex::new(7_u8);
        {
            let guard = mutex.lock().expect("model mutex lock");
            assert_eq!(*guard, 7);
        }

        clear_lock_hook();
        assert_eq!(hook.events(), vec![("acquire", 1), ("release", 1)]);
    }

    #[test]
    fn model_mutex_passthrough_without_hook() {
        let _serial = serial();
        reset_model_mutex_ids_for_model();
        clear_lock_hook();
        clear_spawn_hook();
        let mutex = ModelMutex::new(1_u8);

        *mutex.lock().expect("model mutex lock") = 2;

        assert_eq!(*mutex.lock().expect("model mutex lock"), 2);
    }

    #[test]
    fn model_mutex_blocks_with_probe_only_lock_hook() {
        let _serial = serial();
        reset_model_mutex_ids_for_model();
        clear_lock_hook();
        clear_spawn_hook();
        let hook = Arc::new(RecordingLockHook::new());
        install_lock_hook(hook.clone());

        let mutex = Arc::new(ModelMutex::new(0_u8));
        let (locked_tx, locked_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();

        let first_mutex = Arc::clone(&mutex);
        let first = std::thread::spawn(move || {
            let guard = first_mutex.lock().expect("first lock");
            locked_tx.send(()).expect("send locked");
            release_rx.recv().expect("wait for release");
            drop(guard);
        });

        locked_rx.recv().expect("first worker locked");

        let second_mutex = Arc::clone(&mutex);
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let second = std::thread::spawn(move || {
            let guard = second_mutex
                .lock()
                .expect("second lock blocks then succeeds");
            acquired_tx.send(()).expect("send acquired");
            drop(guard);
        });

        assert!(
            acquired_rx.recv_timeout(Duration::from_millis(25)).is_err(),
            "second worker should block while the first guard is held"
        );
        release_tx.send(()).expect("release first guard");
        acquired_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("second worker acquired after release");

        first.join().expect("first worker");
        second.join().expect("second worker");
        clear_lock_hook();

        assert_eq!(
            hook.events(),
            vec![
                ("acquire", 1),
                ("acquire", 1),
                ("release", 1),
                ("release", 1),
            ]
        );
    }

    #[test]
    fn model_mutex_try_lock_reports_acquire_on_success_only() {
        let _serial = serial();
        reset_model_mutex_ids_for_model();
        clear_lock_hook();
        clear_spawn_hook();
        let hook = Arc::new(RecordingLockHook::new());
        install_lock_hook(hook.clone());

        let mutex = ModelMutex::new(0_u8);
        {
            let guard = mutex.try_lock().expect("uncontended try_lock succeeds");
            // A second try while held must fail without reporting an acquire.
            assert!(matches!(mutex.try_lock(), Err(TryLockError::WouldBlock)));
            drop(guard);
        }

        clear_lock_hook();
        assert_eq!(hook.events(), vec![("acquire", 1), ("release", 1)]);
    }

    #[test]
    fn model_rwlock_read_and_write_route_exclusive_boundary() {
        let _serial = serial();
        reset_model_mutex_ids_for_model();
        clear_lock_hook();
        clear_spawn_hook();
        let hook = Arc::new(RecordingLockHook::new());
        install_lock_hook(hook.clone());

        let rw = ModelRwLock::new(1_u8);
        {
            let r = rw.read().expect("model rwlock read");
            assert_eq!(*r, 1);
        }
        {
            let mut w = rw.write().expect("model rwlock write");
            *w = 2;
        }
        assert_eq!(*rw.read().expect("model rwlock read"), 2);

        clear_lock_hook();
        // read → acquire/release(1), write → acquire/release(1), final read → acquire/release(1).
        assert_eq!(
            hook.events(),
            vec![
                ("acquire", 1),
                ("release", 1),
                ("acquire", 1),
                ("release", 1),
                ("acquire", 1),
                ("release", 1),
            ]
        );
    }

    #[test]
    fn model_rwlock_passthrough_without_hook() {
        let _serial = serial();
        reset_model_mutex_ids_for_model();
        clear_lock_hook();
        clear_spawn_hook();
        let rw = ModelRwLock::new(1_u8);

        *rw.write().expect("model rwlock write") = 5;

        assert_eq!(*rw.read().expect("model rwlock read"), 5);
        assert!(rw.try_write().is_ok());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_task_passes_through_to_tokio_without_a_hook() {
        let receiver = {
            let _serial = serial();
            clear_async_spawn_hook();

            let (sender, receiver) = tokio::sync::oneshot::channel();
            spawn_task(async move {
                sender
                    .send(())
                    .expect("native task receiver must remain open");
            });
            receiver
        };

        tokio::time::timeout(Duration::from_secs(1), receiver)
            .await
            .expect("native tokio task must run")
            .expect("native task must send its completion signal");
    }
}
