// SPDX-License-Identifier: Apache-2.0
//! Runtime seams for annotated Laplace model code.
//!
//! `spawn` routes unit-returning model threads through an installed engine hook.
//! `ModelMutex` routes qualified `std::sync::Mutex` annotated by
//! `#[laplace::model]` through an installed lock hook. With no hook installed,
//! these surfaces delegate to the standard library.
//!
//! P-1 only supports `FnOnce() -> ()` closures. Non-unit spawn returns,
//! `std::thread::Builder`, scoped threads, async task spawning, and unqualified
//! bare `spawn(...)` calls are outside this runtime surface.
//!
//! P-2 only rewrites qualified `std::sync::Mutex` paths. Bare `Mutex`,
//! `sync::Mutex`, `try_lock`, poison recovery helpers, `Condvar::wait`, mapped
//! guards, scoped threads, and unannotated source are outside this runtime
//! surface.

use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::sync::{LockResult, MutexGuard as StdMutexGuard, PoisonError, TryLockError};
use std::thread::JoinHandle;

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

enum JoinMode {
    Std(JoinHandle<()>),
    Engine,
}

/// Join handle returned by [`spawn`].
///
/// Without an installed hook this wraps a real `std::thread::JoinHandle<()>`.
/// With an engine hook installed, join ownership stays with the engine runtime,
/// so [`JoinToken::join`] is a no-op success.
pub struct JoinToken {
    mode: JoinMode,
}

impl JoinToken {
    fn from_std(handle: JoinHandle<()>) -> Self {
        Self {
            mode: JoinMode::Std(handle),
        }
    }

    /// Creates an engine-owned join token.
    ///
    /// Engine hooks return this after handing the closure to their own runtime.
    #[must_use]
    pub const fn engine() -> Self {
        Self {
            mode: JoinMode::Engine,
        }
    }

    /// Waits for a free-tier thread or acknowledges an engine-owned thread.
    ///
    /// # Errors
    ///
    /// Returns the panic payload from the underlying std thread in free tier.
    pub fn join(self) -> std::thread::Result<()> {
        match self.mode {
            JoinMode::Std(handle) => handle.join(),
            JoinMode::Engine => Ok(()),
        }
    }
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

fn spawn_hook() -> Option<Arc<dyn SpawnHook>> {
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

/// Resets deterministic model mutex id allocation for controlled re-execution.
///
/// This is separate from hook installation so free-tier code keeps process-wide
/// distinct resource ids, while the private engine can make each reset replay
/// the same two-resource program shape.
#[doc(hidden)]
pub fn reset_model_mutex_ids_for_model() {
    NEXT_LOCK_RESOURCE_ID.store(1, Ordering::SeqCst);
}

fn lock_hook() -> Option<Arc<dyn LockHook>> {
    LOCK_HOOK
        .get()
        .and_then(|slot| slot.lock().expect("lock hook lock poisoned").clone())
}

/// Spawns a unit-returning model thread.
///
/// If a hook is installed, the closure is routed to that hook. Otherwise it is
/// executed on a normal OS thread via `std::thread::spawn`.
#[must_use]
pub fn spawn<F>(f: F) -> JoinToken
where
    F: FnOnce() + Send + 'static,
{
    if let Some(hook) = spawn_hook() {
        return hook.spawn(Box::new(f));
    }

    JoinToken::from_std(std::thread::spawn(f))
}

/// `std::sync::Mutex<T>` compatible model mutex for annotated code.
pub struct ModelMutex<T: ?Sized> {
    resource: u64,
    inner: StdMutex<T>,
}

impl<T> ModelMutex<T> {
    /// Creates a new model mutex with a distinct process-local resource id.
    pub fn new(t: T) -> Self {
        Self {
            resource: NEXT_LOCK_RESOURCE_ID.fetch_add(1, Ordering::SeqCst),
            inner: StdMutex::new(t),
        }
    }
}

impl<T: ?Sized> ModelMutex<T> {
    /// Acquires the mutex.
    ///
    /// The signature mirrors `std::sync::Mutex::lock`, allowing annotated
    /// source to keep `.lock().unwrap()` unchanged. When a hook is installed,
    /// the acquire boundary is reported before the underlying lock attempt and
    /// the release boundary is reported when the returned guard is dropped.
    pub fn lock(&self) -> LockResult<ModelMutexGuard<'_, T>> {
        let hook = lock_hook();
        if let Some(hook) = &hook {
            hook.acquire(self.resource);
        }

        if hook.is_some() {
            return self.inner.try_lock().map_or_else(
                |err| match err {
                    TryLockError::Poisoned(poisoned) => Err(PoisonError::new(ModelMutexGuard {
                        inner: Some(poisoned.into_inner()),
                        resource: self.resource,
                    })),
                    TryLockError::WouldBlock => {
                        panic!("laplace-rt lock hook granted a contended mutex")
                    }
                },
                |inner| {
                    Ok(ModelMutexGuard {
                        inner: Some(inner),
                        resource: self.resource,
                    })
                },
            );
        }

        self.inner
            .lock()
            .map(|inner| ModelMutexGuard {
                inner: Some(inner),
                resource: self.resource,
            })
            .map_err(|err| {
                PoisonError::new(ModelMutexGuard {
                    inner: Some(err.into_inner()),
                    resource: self.resource,
                })
            })
    }
}

/// Guard returned by [`ModelMutex::lock`].
pub struct ModelMutexGuard<'a, T: ?Sized> {
    inner: Option<StdMutexGuard<'a, T>>,
    resource: u64,
}

impl<T: ?Sized> Deref for ModelMutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner.as_deref().expect("model mutex guard is present")
    }
}

impl<T: ?Sized> DerefMut for ModelMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner
            .as_deref_mut()
            .expect("model mutex guard is present")
    }
}

impl<T: ?Sized> Drop for ModelMutexGuard<'_, T> {
    fn drop(&mut self) {
        if self.inner.is_some() && !std::thread::panicking() {
            if let Some(hook) = lock_hook() {
                hook.release(self.resource);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

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
        reset_model_mutex_ids_for_model();
        clear_lock_hook();
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
        reset_model_mutex_ids_for_model();
        clear_lock_hook();
        let mutex = ModelMutex::new(1_u8);

        *mutex.lock().expect("model mutex lock") = 2;

        assert_eq!(*mutex.lock().expect("model mutex lock"), 2);
    }
}
