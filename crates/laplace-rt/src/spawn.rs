// SPDX-License-Identifier: Apache-2.0
//! Model-thread spawn seam.
//!
//! [`spawn`] routes a unit-returning model thread through an installed
//! [`SpawnHook`](crate::hooks::SpawnHook); [`spawn_task`] similarly routes a
//! fire-and-forget async future through an [`AsyncSpawnHook`](crate::hooks::AsyncSpawnHook).
//! With no hook installed, the two seams use their native thread and tokio
//! implementations respectively.

use std::future::Future;
use std::thread::JoinHandle;

use crate::hooks::{async_spawn_hook, spawn_hook};

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

/// Spawns a fire-and-forget async task.
///
/// If an async spawn hook is installed, the future is routed to that hook and
/// its output is discarded. Otherwise it is delegated to `tokio::spawn`.
pub fn spawn_task<T, F>(future: F)
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
{
    if let Some(hook) = async_spawn_hook() {
        hook.spawn_task(Box::pin(async move {
            let _ = future.await;
        }));
    } else {
        drop(tokio::spawn(future));
    }
}
