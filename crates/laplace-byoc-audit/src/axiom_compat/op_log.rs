// SPDX-License-Identifier: Apache-2.0
//! Thread-local operation log for DPOR harness integration.
//!
//! Every `AxiomSemaphore` operation (acquire / try_acquire / release) pushes
//! a [`SyncOp`] entry into a per-thread log.  The DPOR harness reads from this
//! log via [`drain_ops`] to obtain the `(Operation, ResourceId)` sequence for
//! each virtual thread.
//!
//! # Thread safety
//!
//! The log is `thread_local!` and therefore requires no synchronisation.
//! Each OS thread that calls `AxiomSemaphore` methods accumulates its own
//! independent log.

use std::cell::RefCell;

/// The kind of synchronisation operation that was recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncOpKind {
    /// Blocking acquire (maps to `Operation::Request` in DPOR).
    Acquire,
    /// Non-blocking try-acquire (maps to `Operation::Request` in DPOR).
    TryAcquire,
    /// Release / add_permits (maps to `Operation::Release` in DPOR).
    Release,
}

/// A single recorded synchronisation operation.
#[derive(Debug, Clone)]
pub struct SyncOp {
    /// What kind of operation was performed.
    pub kind: SyncOpKind,
    /// Resource ID of the semaphore that was operated on.
    pub resource_id: usize,
}

thread_local! {
    static OP_LOG: RefCell<Vec<SyncOp>> = const { RefCell::new(Vec::new()) };
}

/// Record a synchronisation operation in the current thread's log.
///
/// Called by `AxiomSemaphore` methods.
#[inline]
pub fn record_op(kind: SyncOpKind, resource_id: usize) {
    OP_LOG.with(|log| {
        log.borrow_mut().push(SyncOp { kind, resource_id });
    });
}

/// Drain all recorded operations from the current thread's log.
///
/// Called by the DPOR harness `op_provider` to retrieve what the thread
/// has done since the last call.
#[inline]
pub fn drain_ops() -> Vec<SyncOp> {
    OP_LOG.with(|log| log.borrow_mut().drain(..).collect())
}

/// Peek at (but do not drain) the current thread's log.
#[inline]
pub fn peek_ops() -> Vec<SyncOp> {
    OP_LOG.with(|log| log.borrow().clone())
}

/// Clear the current thread's log without returning the entries.
#[inline]
pub fn clear_ops() {
    OP_LOG.with(|log| log.borrow_mut().clear());
}
