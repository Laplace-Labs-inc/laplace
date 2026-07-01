// SPDX-License-Identifier: Apache-2.0
//! Model-thread spawn seam.
//!
//! [`spawn`] routes a unit-returning model thread through an installed
//! [`SpawnHook`](crate::hooks::SpawnHook); with no hook installed it runs on a
//! normal OS thread.

use std::thread::JoinHandle;

use crate::hooks::spawn_hook;

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
