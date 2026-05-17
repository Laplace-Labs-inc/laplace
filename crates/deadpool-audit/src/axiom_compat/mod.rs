//! AxiomSync — surgical patch layer replacing `tokio::sync` in deadpool.
//!
//! This module provides drop-in replacements for the two `tokio::sync`
//! primitives that deadpool's managed pool actually uses:
//!
//! * [`AxiomSemaphore`]  ← replaces `tokio::sync::Semaphore`
//! * [`AxiomTryAcquireError`] ← replaces `tokio::sync::TryAcquireError`
//!
//! Both types preserve the exact API surface that `managed/mod.rs` calls,
//! so the patched pool compiles and runs identically to the original.
//!
//! ## Operation Log
//!
//! When the `axiom` feature is enabled each `acquire()` and `add_permits()`
//! call additionally pushes an entry to the **thread-local [`op_log`]**:
//!
//! ```text
//! acquire()      → SyncOpKind::Acquire  (maps to Operation::Request)
//! add_permits(n) → SyncOpKind::Release  (maps to Operation::Release, one per permit)
//! ```
//!
//! The DPOR harness reads from `op_log::drain_ops()` to build the
//! `(Operation, ResourceId)` tuples consumed by `KiDporScheduler::expand_current`.

pub mod op_log;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::SemaphorePermit;

/// Global counter for allocating unique resource IDs to pool semaphores.
static NEXT_RESOURCE_ID: AtomicUsize = AtomicUsize::new(0);

// ── AxiomTryAcquireError ──────────────────────────────────────────────────────

/// Drop-in replacement for `tokio::sync::TryAcquireError`.
///
/// Mirrors the two variants that deadpool actually pattern-matches:
/// `Closed` and `NoPermits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxiomTryAcquireError {
    /// The semaphore has been closed.
    Closed,
    /// No permits are currently available.
    NoPermits,
}

// ── AxiomPermit ──────────────────────────────────────────────────────────────

/// Drop-in replacement for `tokio::sync::SemaphorePermit`.
///
/// Wraps a real `SemaphorePermit` so actual blocking semantics are preserved.
/// The `forget()` method consumes the permit without releasing it, exactly
/// as `tokio::sync::SemaphorePermit::forget` does.
pub struct AxiomPermit<'a> {
    inner: SemaphorePermit<'a>,
    /// Resource ID of the parent semaphore (for op-log completions).
    #[allow(dead_code)]
    resource_id: usize,
}

impl<'a> AxiomPermit<'a> {
    /// Consume this permit without returning it to the semaphore.
    ///
    /// Mirrors `tokio::sync::SemaphorePermit::forget`.
    pub fn forget(self) {
        self.inner.forget()
    }
}

// ── AxiomSemaphore ────────────────────────────────────────────────────────────

/// Drop-in replacement for `tokio::sync::Semaphore`.
///
/// # Surgical patch notes
///
/// In `deadpool/src/managed/mod.rs` line 76 the original import:
///
/// ```ignore
/// use tokio::sync::{Semaphore, TryAcquireError};
/// ```
///
/// is replaced (via `#[cfg(feature = "axiom")]` guards) with:
///
/// ```ignore
/// use crate::axiom_compat::{
///     AxiomSemaphore       as Semaphore,
///     AxiomTryAcquireError as TryAcquireError,
/// };
/// ```
///
/// All other code in `managed/mod.rs` is **unchanged**.
///
/// # Dual-mode behaviour
///
/// The `AxiomSemaphore` behaves identically to `tokio::sync::Semaphore` for
/// real execution (backed by a real tokio semaphore).  When the `axiom` Cargo
/// feature is enabled, every `acquire()` and `add_permits()` call additionally
/// records an entry to the thread-local [`op_log`] so that the DPOR harness
/// can map each real blocking point to an `(Operation, ResourceId)` pair.
pub struct AxiomSemaphore {
    /// Real tokio semaphore — provides actual blocking / waking.
    inner: tokio::sync::Semaphore,
    /// Unique ID assigned at construction; stable across calls.
    pub resource_id: usize,
    /// Mirrors `tokio::sync::Semaphore::is_closed`.
    closed: AtomicBool,
}

impl AxiomSemaphore {
    /// Create a new semaphore with `permits` initial permits.
    ///
    /// Mirrors `tokio::sync::Semaphore::new`.
    pub fn new(permits: usize) -> Self {
        let resource_id = NEXT_RESOURCE_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            inner: tokio::sync::Semaphore::new(permits),
            resource_id,
            closed: AtomicBool::new(false),
        }
    }

    /// Attempt to acquire a permit without blocking.
    ///
    /// Mirrors `tokio::sync::Semaphore::try_acquire`.
    /// Records `SyncOpKind::TryAcquire` to the op log on success.
    pub fn try_acquire(&self) -> Result<AxiomPermit<'_>, AxiomTryAcquireError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(AxiomTryAcquireError::Closed);
        }
        match self.inner.try_acquire() {
            Ok(permit) => {
                op_log::record_op(op_log::SyncOpKind::TryAcquire, self.resource_id);
                Ok(AxiomPermit {
                    inner: permit,
                    resource_id: self.resource_id,
                })
            }
            Err(tokio::sync::TryAcquireError::Closed) => Err(AxiomTryAcquireError::Closed),
            Err(tokio::sync::TryAcquireError::NoPermits) => Err(AxiomTryAcquireError::NoPermits),
        }
    }

    /// Acquire a permit, waiting if none are available.
    ///
    /// Mirrors `tokio::sync::Semaphore::acquire`.
    /// Records `SyncOpKind::Acquire` to the op log **before** the await point,
    /// capturing the moment the thread would block in a real execution.
    pub async fn acquire(&self) -> Result<AxiomPermit<'_>, tokio::sync::AcquireError> {
        // ── DPOR hook: record the blocking acquire attempt ────────────────────
        op_log::record_op(op_log::SyncOpKind::Acquire, self.resource_id);
        // ── Real execution: wait for the permit ───────────────────────────────
        let permit = self.inner.acquire().await?;
        Ok(AxiomPermit {
            inner: permit,
            resource_id: self.resource_id,
        })
    }

    /// Add `n` permits to the semaphore.
    ///
    /// Mirrors `tokio::sync::Semaphore::add_permits`.
    /// Records one `SyncOpKind::Release` per permit added.
    pub fn add_permits(&self, n: usize) {
        // ── DPOR hook: record each release ───────────────────────────────────
        for _ in 0..n {
            op_log::record_op(op_log::SyncOpKind::Release, self.resource_id);
        }
        // ── Real execution: wake up waiters ───────────────────────────────────
        self.inner.add_permits(n);
    }

    /// Close the semaphore, failing all pending and future acquires.
    ///
    /// Mirrors `tokio::sync::Semaphore::close`.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.inner.close();
    }

    /// Returns `true` if the semaphore has been closed.
    ///
    /// Mirrors `tokio::sync::Semaphore::is_closed`.
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
}

impl std::fmt::Debug for AxiomSemaphore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AxiomSemaphore")
            .field("resource_id", &self.resource_id)
            .field("available_permits", &self.inner.available_permits())
            .field("closed", &self.closed.load(Ordering::Relaxed))
            .finish()
    }
}
