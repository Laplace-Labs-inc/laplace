//! Scheduler type definitions.
//!
//! Canonical types for the Laplace thread scheduling subsystem.
//! Corresponds to the TLA+ `SchedulerOracle` specification.

use std::fmt;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Scheduler-domain thread identifier.
///
/// Identifies a single simulated thread within the scheduler. Values are
/// zero-based indices in `[0, max_threads)`.
///
/// # Note
/// This type is distinct from `resource::ThreadId` and `tracing::ThreadId`.
///
/// # TLA+ Correspondence
/// Element of the `Threads` constant set in `SchedulerOracle.tla`.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Scheduler",
        link = "LEP-0010-laplace-interfaces-scheduler_contracts"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ThreadId(pub usize);

impl ThreadId {
    /// Create a new `ThreadId` from a zero-based index.
    ///
    /// - `id`: Zero-based thread index.
    ///
    /// Returns `ThreadId(id)`.
    #[inline(always)]
    pub const fn new(id: usize) -> Self {
        Self(id)
    }

    /// Return the raw `usize` index of this thread.
    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

impl fmt::Display for ThreadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Thread({})", self.0)
    }
}

/// Logical task identifier.
///
/// Identifies a unit of work scheduled for a thread. Multiple events may be
/// derived from a single task, but `TaskId` tracks the conceptual task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaskId(pub usize);

impl TaskId {
    /// Create a new `TaskId` from a zero-based index.
    ///
    /// - `id`: Unique task index.
    ///
    /// Returns `TaskId(id)`.
    #[inline(always)]
    pub const fn new(id: usize) -> Self {
        Self(id)
    }

    /// Return the raw `usize` index of this task.
    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Task({})", self.0)
    }
}

/// Lifecycle state of a scheduled thread.
///
/// # TLA+ Correspondence
/// ```tla
/// ThreadStates == {"RUNNABLE", "BLOCKED", "COMPLETED"}
/// ```
///
/// # Valid Transitions
/// - `Runnable → Blocked` (thread awaits a resource)
/// - `Blocked → Runnable` (resource becomes available)
/// - `Runnable → Completed` (thread finishes execution)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThreadState {
    /// Thread is eligible to execute events.
    Runnable,

    /// Thread is waiting for a resource or dependency.
    Blocked,

    /// Thread has finished execution.
    Completed,
}

impl ThreadState {
    /// Return `true` if the thread is in the `Runnable` state.
    #[inline(always)]
    pub const fn is_runnable(self) -> bool {
        matches!(self, ThreadState::Runnable)
    }

    /// Return `true` if the thread is in the `Blocked` state.
    #[inline(always)]
    pub const fn is_blocked(self) -> bool {
        matches!(self, ThreadState::Blocked)
    }

    /// Return `true` if the thread is in the `Completed` state.
    #[inline(always)]
    pub const fn is_completed(self) -> bool {
        matches!(self, ThreadState::Completed)
    }
}

impl fmt::Display for ThreadState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ThreadState::Runnable => write!(f, "RUNNABLE"),
            ThreadState::Blocked => write!(f, "BLOCKED"),
            ThreadState::Completed => write!(f, "COMPLETED"),
        }
    }
}

/// Event-selection strategy for the scheduler.
///
/// # TLA+ Correspondence
/// ```tla
/// CONSTANTS Strategy
/// ASSUME Strategy \in {"PRODUCTION", "VERIFICATION"}
/// ```
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Scheduler",
        link = "LEP-0010-laplace-interfaces-scheduler_contracts"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulingStrategy {
    /// Deterministic selection ordered by time, Lamport clock, and event ID.
    Production,

    /// Non-deterministic selection for exhaustive state-space exploration.
    Verification,
}

impl SchedulingStrategy {
    /// Return `true` if this is the `Production` strategy.
    #[inline(always)]
    pub const fn is_production(self) -> bool {
        matches!(self, SchedulingStrategy::Production)
    }

    /// Return `true` if this is the `Verification` strategy.
    #[inline(always)]
    pub const fn is_verification(self) -> bool {
        matches!(self, SchedulingStrategy::Verification)
    }
}

impl fmt::Display for SchedulingStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SchedulingStrategy::Production => write!(f, "PRODUCTION"),
            SchedulingStrategy::Verification => write!(f, "VERIFICATION"),
        }
    }
}

/// Error variants for scheduler operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchedulerError {
    /// The supplied `thread_id` exceeds the backend's `max_threads`.
    InvalidThreadId {
        /// The out-of-range thread identifier.
        thread_id: ThreadId,
        /// Maximum allowed thread index.
        max_threads: usize,
    },

    /// The thread is not in the state required for the operation.
    InvalidThreadState {
        /// Thread in the wrong state.
        thread_id: ThreadId,
        /// State the thread is actually in.
        current_state: ThreadState,
        /// State that was required.
        expected_state: ThreadState,
    },

    /// The event queue is at capacity and cannot accept more events.
    QueueFull {
        /// Maximum queue capacity.
        max_events: usize,
        /// Event count that was attempted.
        attempted: usize,
    },

    /// Scheduling an event at `current_time_ns + delay_ns` would overflow the max time.
    TimeOverflow {
        /// Current virtual time in nanoseconds.
        current_time_ns: u64,
        /// Requested delay in nanoseconds.
        delay_ns: u64,
        /// Maximum virtual time allowed.
        max_time_ns: u64,
    },

    /// No runnable events exist in the current scheduling cycle.
    NoRunnableEvents,
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SchedulerError::InvalidThreadId {
                thread_id,
                max_threads,
            } => write!(
                f,
                "Invalid thread ID: {} (max threads: {})",
                thread_id, max_threads
            ),
            SchedulerError::InvalidThreadState {
                thread_id,
                current_state,
                expected_state,
            } => write!(
                f,
                "Thread {} in invalid state: {} (expected: {})",
                thread_id, current_state, expected_state
            ),
            SchedulerError::QueueFull {
                max_events,
                attempted,
            } => write!(
                f,
                "Event queue full (max: {}, attempted: {})",
                max_events, attempted
            ),
            SchedulerError::TimeOverflow {
                current_time_ns,
                delay_ns,
                max_time_ns,
            } => write!(
                f,
                "Time overflow: {} + {} would exceed max time {}",
                current_time_ns, delay_ns, max_time_ns
            ),
            SchedulerError::NoRunnableEvents => write!(f, "No runnable events available"),
        }
    }
}

impl std::error::Error for SchedulerError {}
