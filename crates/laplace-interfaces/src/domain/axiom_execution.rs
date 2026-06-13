// SPDX-License-Identifier: Apache-2.0
//! Axiom Execution v2 op-source contracts.
//!
//! This module contains the public contract that lets the Ki-DPOR core consume
//! operations from either a static replay trace or a controlled re-execution
//! runtime without depending on either implementation.

use crate::domain::resource::{ResourceId, ThreadId};
use serde::{Deserialize, Serialize};
use std::ffi::c_void;

/// Ki-DPOR-compatible synchronization operation.
///
/// The variants intentionally mirror the seven `laplace-dpor::Operation`
/// variants. They live in `laplace-interfaces` to avoid a circular dependency
/// between the public interface crate and the DPOR implementation crate.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AxiomOperation {
    /// Exclusive lock acquisition.
    Request = 0,
    /// Exclusive lock release.
    Release = 1,
    /// Shared lock acquisition.
    SharedRequest = 2,
    /// Shared lock release.
    SharedRelease = 3,
    /// Shared memory read.
    Read = 4,
    /// Exclusive memory write.
    Write = 5,
    /// Read-modify-write memory operation.
    ReadWrite = 6,
}

/// Determinism guarantee declared by an [`ExecutionSource`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeterminismClass {
    /// The source is deterministic for the same seed, code, and config.
    FullyDeterministic = 0,
    /// The source is deterministic when declared external inputs are identical.
    DeterministicWithDeclaredInputs = 1,
    /// The source may include unshadowed inputs; results require warnings.
    BestEffort = 2,
}

impl Default for DeterminismClass {
    fn default() -> Self {
        Self::FullyDeterministic
    }
}

/// Reserved async yield point kind.
///
/// Axiom Execution v2 P1 does not interpret async yields. The enum is present so
/// P2 can activate the contract without changing the `StepOutcome` shape.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum YieldKind {
    /// A future poll yielded.
    Poll = 0,
    /// A parked task was woken by another task.
    Wake = 1,
    /// A task was cancelled or dropped.
    Cancelled = 2,
}

/// Captured user panic metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanicReport {
    /// Thread that observed the panic.
    pub thread: ThreadId,
    /// Stable panic message. File paths and source snippets must not be stored here.
    pub message: String,
}

impl PanicReport {
    /// Creates a panic report with a stable message.
    pub fn new(thread: ThreadId, message: impl Into<String>) -> Self {
        Self {
            thread,
            message: message.into(),
        }
    }
}

/// Outcome produced by advancing one model thread to the next boundary.
#[repr(C, u8)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StepOutcome {
    /// A Ki-DPOR operation occurred.
    Op(AxiomOperation, ResourceId),
    /// The thread entered a resource wait.
    Blocked(ResourceId),
    /// The thread completed all work.
    Finished,
    /// The user code panicked and the runner caught it.
    Panicked(PanicReport),
    /// Reserved async yield signal for P2.
    Yield(YieldKind),
}

/// Bitset of currently runnable model threads.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AxiomThreadSet(u64);

impl AxiomThreadSet {
    /// Creates an empty thread set.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Creates a thread set from raw bits.
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// Returns the raw bit representation.
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Returns true when `thread` is present in the set.
    pub fn contains(self, thread: ThreadId) -> bool {
        thread.as_usize() < 64 && (self.0 & (1_u64 << thread.as_usize())) != 0
    }

    /// Returns a new set that includes `thread`.
    pub fn with(self, thread: ThreadId) -> Self {
        if thread.as_usize() >= 64 {
            return self;
        }
        Self(self.0 | (1_u64 << thread.as_usize()))
    }
}

/// Errors returned by an operation source before a new schedule starts.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    /// The requested thread is outside the source thread count.
    #[error("invalid thread {thread:?}; thread_count={thread_count}")]
    InvalidThread {
        /// Invalid thread ID.
        thread: ThreadId,
        /// Number of threads exposed by the source.
        thread_count: usize,
    },

    /// The source cannot provide deterministic replay for this run.
    #[error("non-deterministic input is not shadowed: {description}")]
    NonDeterministicInput {
        /// Stable description of the unshadowed input.
        description: String,
    },

    /// The source does not support the requested operation in P1.
    #[error("unsupported execution source feature: {feature}")]
    Unsupported {
        /// Feature name that is not supported.
        feature: &'static str,
    },
}

/// Ki-DPOR op supply contract.
///
/// Implementations may be replay-backed or live controlled re-execution
/// runtimes. The Ki-DPOR core consumes only this contract through an adapter.
pub trait ExecutionSource {
    /// Starts a new schedule exploration from the beginning.
    fn reset(&mut self) -> Result<(), SourceError>;

    /// Advances `thread` to the next synchronization boundary.
    fn step(&mut self, thread: ThreadId) -> StepOutcome;

    /// Returns the set of threads that may currently be stepped.
    fn enabled(&self) -> AxiomThreadSet;

    /// Returns the number of model threads exposed by this source.
    fn thread_count(&self) -> usize;

    /// Returns the source's determinism declaration.
    fn determinism_class(&self) -> DeterminismClass;
}

/// FFI-safe source error code for vtable wrappers.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceErrorCode {
    /// No error occurred.
    Ok = 0,
    /// Invalid thread ID.
    InvalidThread = 1,
    /// Non-deterministic input was observed.
    NonDeterministicInput = 2,
    /// Unsupported feature was requested.
    Unsupported = 3,
}

/// FFI-safe step outcome tag.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcomeTag {
    /// Operation outcome.
    Op = 0,
    /// Blocked outcome.
    Blocked = 1,
    /// Finished outcome.
    Finished = 2,
    /// Panicked outcome.
    Panicked = 3,
    /// Yield outcome.
    Yield = 4,
}

/// FFI-safe representation of a step outcome.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepOutcomeFfi {
    /// Outcome tag.
    pub tag: StepOutcomeTag,
    /// Operation value when `tag == Op`.
    pub operation: AxiomOperation,
    /// Resource index for operation or blocked outcomes.
    pub resource: usize,
    /// Yield kind when `tag == Yield`.
    pub yield_kind: YieldKind,
}

/// FFI vtable shape for execution sources.
///
/// W1 freezes the signatures only. Concrete vtable construction is owned by the
/// source implementation crates.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExecutionSourceVTable {
    /// Starts a new schedule exploration.
    pub reset: unsafe extern "C" fn(*mut c_void) -> SourceErrorCode,
    /// Advances the supplied thread by one boundary.
    pub step: unsafe extern "C" fn(*mut c_void, usize) -> StepOutcomeFfi,
    /// Returns raw enabled-thread bits.
    pub enabled: unsafe extern "C" fn(*const c_void) -> u64,
    /// Returns the source thread count.
    pub thread_count: unsafe extern "C" fn(*const c_void) -> usize,
    /// Returns the determinism class.
    pub determinism_class: unsafe extern "C" fn(*const c_void) -> DeterminismClass,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_set_is_immutable() {
        let empty = AxiomThreadSet::empty();
        let with_t1 = empty.with(ThreadId::new(1));

        assert!(!empty.contains(ThreadId::new(1)));
        assert!(with_t1.contains(ThreadId::new(1)));
        assert_eq!(with_t1.bits(), 0b10);
    }

    #[test]
    fn operation_discriminants_are_frozen() {
        assert_eq!(AxiomOperation::Request as u8, 0);
        assert_eq!(AxiomOperation::Release as u8, 1);
        assert_eq!(AxiomOperation::SharedRequest as u8, 2);
        assert_eq!(AxiomOperation::SharedRelease as u8, 3);
        assert_eq!(AxiomOperation::Read as u8, 4);
        assert_eq!(AxiomOperation::Write as u8, 5);
        assert_eq!(AxiomOperation::ReadWrite as u8, 6);
    }

    #[test]
    fn async_yield_abi_layout_is_frozen() {
        assert_eq!(YieldKind::Poll as u8, 0);
        assert_eq!(YieldKind::Wake as u8, 1);
        assert_eq!(YieldKind::Cancelled as u8, 2);
        assert_eq!(std::mem::size_of::<StepOutcome>(), 40);
        assert_eq!(std::mem::align_of::<StepOutcome>(), 8);
        assert_eq!(std::mem::size_of::<StepOutcomeFfi>(), 24);
        assert_eq!(std::mem::align_of::<StepOutcomeFfi>(), 8);
    }
}
