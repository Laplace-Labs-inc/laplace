// SPDX-License-Identifier: Apache-2.0
//! Process-local engine hooks and deterministic model resource-id allocation.
//!
//! The engine installs a [`SpawnHook`] and/or [`LockHook`] to take control of a
//! model run; with no hook installed the seams in [`crate::spawn`],
//! [`crate::mutex`], and [`crate::rwlock`] delegate to the standard library.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use crate::spawn::JoinToken;

static SPAWN_HOOK: OnceLock<StdMutex<Option<Arc<dyn SpawnHook>>>> = OnceLock::new();
static LOCK_HOOK: OnceLock<StdMutex<Option<Arc<dyn LockHook>>>> = OnceLock::new();
static NEXT_LOCK_RESOURCE_ID: AtomicU64 = AtomicU64::new(1);
static ASYNC_LOCK_HOOK: OnceLock<StdMutex<Option<Arc<dyn AsyncLockHook>>>> = OnceLock::new();
static NEXT_ASYNC_LOCK_RESOURCE_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_ASYNC_LOCK_WAITER_ID: AtomicU64 = AtomicU64::new(1);

/// Engine-installed surface for creating one controlled model thread.
pub trait SpawnHook: Send + Sync {
    /// Creates one model thread under engine control.
    fn spawn(&self, f: Box<dyn FnOnce() + Send + 'static>) -> JoinToken;
}

/// Engine- or probe-installed surface for model mutex boundaries.
pub trait LockHook: Send + Sync {
    /// Reports a model mutex acquisition boundary.
    fn acquire(&self, resource: u64);

    /// Reports a model mutex release boundary.
    fn release(&self, resource: u64);
}

/// Engine- or probe-installed surface for model async mutex boundaries.
///
/// Mirrors [`LockHook`] but for the `tokio::sync::Mutex`-compatible async seam
/// ([`crate::ModelAsyncMutex`]), which additionally distinguishes a queued
/// waiter (a live, unpolled `lock()` future) from an acquired guard.
pub trait AsyncLockHook: Send + Sync {
    /// Reports that a `lock()` future's first poll found contention and
    /// queued behind the current holder.
    fn requested(&self, resource: u64, waiter: u64);

    /// Reports a guard acquisition, either immediately (uncontended) or by
    /// resolving a previously queued waiter.
    fn acquired(&self, resource: u64, waiter: u64);

    /// Reports a model async mutex release boundary.
    fn released(&self, resource: u64);

    /// Reports that a queued-but-unacquired `lock()` future was dropped
    /// (cancelled) before it resolved.
    fn waiter_dropped(&self, resource: u64, waiter: u64);
}

/// Installs or replaces the process-local spawn hook.
///
/// # Panics
///
/// Panics if the internal hook registry mutex is poisoned.
pub fn install_spawn_hook(hook: Arc<dyn SpawnHook>) {
    let slot = SPAWN_HOOK.get_or_init(|| StdMutex::new(None));
    *slot.lock().expect("spawn hook lock poisoned") = Some(hook);
}

/// Clears the process-local spawn hook.
///
/// # Panics
///
/// Panics if the internal hook registry mutex is poisoned.
pub fn clear_spawn_hook() {
    if let Some(slot) = SPAWN_HOOK.get() {
        *slot.lock().expect("spawn hook lock poisoned") = None;
    }
}

pub(crate) fn spawn_hook() -> Option<Arc<dyn SpawnHook>> {
    SPAWN_HOOK
        .get()
        .and_then(|slot| slot.lock().expect("spawn hook lock poisoned").clone())
}

/// Installs or replaces the process-local lock hook.
///
/// # Panics
///
/// Panics if the internal hook registry mutex is poisoned.
pub fn install_lock_hook(hook: Arc<dyn LockHook>) {
    let slot = LOCK_HOOK.get_or_init(|| StdMutex::new(None));
    *slot.lock().expect("lock hook lock poisoned") = Some(hook);
}

/// Clears the process-local lock hook.
///
/// # Panics
///
/// Panics if the internal hook registry mutex is poisoned.
pub fn clear_lock_hook() {
    if let Some(slot) = LOCK_HOOK.get() {
        *slot.lock().expect("lock hook lock poisoned") = None;
    }
}

pub(crate) fn lock_hook() -> Option<Arc<dyn LockHook>> {
    LOCK_HOOK
        .get()
        .and_then(|slot| slot.lock().expect("lock hook lock poisoned").clone())
}

/// Resets deterministic model mutex id allocation for controlled re-execution.
///
/// This is separate from hook installation so free-tier code keeps process-wide
/// distinct resource ids, while the private engine can make each reset replay
/// the same two-resource program shape.
#[doc(hidden)]
pub fn reset_model_mutex_ids_for_model() {
    NEXT_LOCK_RESOURCE_ID.store(1, Ordering::SeqCst);
}

/// Allocates the next distinct process-local model resource id.
pub(crate) fn next_lock_resource_id() -> u64 {
    NEXT_LOCK_RESOURCE_ID.fetch_add(1, Ordering::SeqCst)
}

/// Installs or replaces the process-local async lock hook.
///
/// # Panics
///
/// Panics if the internal hook registry mutex is poisoned.
pub fn install_async_lock_hook(hook: Arc<dyn AsyncLockHook>) {
    let slot = ASYNC_LOCK_HOOK.get_or_init(|| StdMutex::new(None));
    *slot.lock().expect("async lock hook lock poisoned") = Some(hook);
}

/// Clears the process-local async lock hook.
///
/// # Panics
///
/// Panics if the internal hook registry mutex is poisoned.
pub fn clear_async_lock_hook() {
    if let Some(slot) = ASYNC_LOCK_HOOK.get() {
        *slot.lock().expect("async lock hook lock poisoned") = None;
    }
}

pub(crate) fn async_lock_hook() -> Option<Arc<dyn AsyncLockHook>> {
    ASYNC_LOCK_HOOK
        .get()
        .and_then(|slot| slot.lock().expect("async lock hook lock poisoned").clone())
}

/// Resets deterministic model async mutex resource-id and waiter-id
/// allocation for controlled re-execution.
///
/// This is separate from hook installation so free-tier code keeps
/// process-wide distinct resource/waiter ids, while the private engine can
/// make each reset replay the same resource/waiter shape.
#[doc(hidden)]
pub fn reset_model_async_mutex_ids_for_model() {
    NEXT_ASYNC_LOCK_RESOURCE_ID.store(1, Ordering::SeqCst);
    NEXT_ASYNC_LOCK_WAITER_ID.store(1, Ordering::SeqCst);
}

/// Allocates the next distinct process-local model async lock resource id.
pub(crate) fn next_async_lock_resource_id() -> u64 {
    NEXT_ASYNC_LOCK_RESOURCE_ID.fetch_add(1, Ordering::SeqCst)
}

/// Allocates the next distinct process-local model async lock waiter id, one
/// per `lock()` future call (not per task — see [`crate::ModelAsyncLock`]).
pub(crate) fn next_async_lock_waiter_id() -> u64 {
    NEXT_ASYNC_LOCK_WAITER_ID.fetch_add(1, Ordering::SeqCst)
}
