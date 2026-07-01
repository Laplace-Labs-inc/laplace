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

/// Installs or replaces the process-local spawn hook.
pub fn install_spawn_hook(hook: Arc<dyn SpawnHook>) {
    let slot = SPAWN_HOOK.get_or_init(|| StdMutex::new(None));
    *slot.lock().expect("spawn hook lock poisoned") = Some(hook);
}

/// Clears the process-local spawn hook.
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
pub fn install_lock_hook(hook: Arc<dyn LockHook>) {
    let slot = LOCK_HOOK.get_or_init(|| StdMutex::new(None));
    *slot.lock().expect("lock hook lock poisoned") = Some(hook);
}

/// Clears the process-local lock hook.
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
