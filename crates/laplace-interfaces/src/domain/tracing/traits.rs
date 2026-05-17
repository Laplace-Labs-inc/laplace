//! Tracer backend trait definition.
//!
//! Defines the abstract interface that all event-storage backends must implement.
//! The `TraceEngine` in `laplace-core` programs to this trait, enabling production
//! (heap-allocated, high-capacity) and verification (stack-allocated, fixed-size)
//! backends to be swapped at compile time with zero runtime overhead.

use super::types::{LamportTimestamp, SimulationEvent};
use std::fmt;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Error variants for tracing operations.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Tracing",
        link = "LEP-0014-laplace-interfaces-deterministic_tracing"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TracingError {
    /// The event buffer has reached its maximum capacity.
    BufferFull,

    /// A causality violation was detected — timestamp regression on a thread.
    CausalityViolation {
        /// Minimum timestamp expected based on prior events from the same thread.
        expected_min: LamportTimestamp,
        /// Timestamp that was actually received (violates monotonicity).
        received: LamportTimestamp,
    },

    /// The supplied thread ID is out of the valid range `[0, MAX_THREADS)`.
    InvalidThreadId(u32),
}

impl fmt::Display for TracingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TracingError::BufferFull => write!(f, "Event buffer is full"),
            TracingError::CausalityViolation {
                expected_min,
                received,
            } => write!(
                f,
                "Causality violation: expected timestamp >= {}, got {}",
                expected_min.0, received.0
            ),
            TracingError::InvalidThreadId(tid) => write!(f, "Invalid thread ID: {}", tid),
        }
    }
}

impl std::error::Error for TracingError {}

/// Abstract interface for simulation event storage.
///
/// Implement this trait to provide a concrete backend for `TraceEngine<B: TracerBackend>`.
///
/// # Safety Contract
///
/// Implementations must maintain:
/// - Events are retrievable by the order they were appended.
/// - `global_timestamp()` is always the maximum timestamp seen across all events.
/// - Index `i` in `get_event(i)` returns the event appended at position `i`.
///
/// # Implementing
///
/// `ProductionBackend` (heap-allocated `Vec`) and `VerificationBackend`
/// (stack-allocated fixed array) implement this trait in `laplace-core`.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Tracing",
        link = "LEP-0014-laplace-interfaces-deterministic_tracing"
    )
)]
pub trait TracerBackend: Send + Sync + fmt::Debug {
    /// Return the maximum number of events this backend can hold.
    ///
    /// Used by `TraceEngine` to enforce capacity limits before appending.
    fn max_events(&self) -> usize;

    /// Append `event` to the trace, assigning it the next sequential index.
    ///
    /// - `event`: The [`SimulationEvent`] to record.
    ///
    /// Returns `Ok(())` on success, `Err(TracingError::BufferFull)` if at capacity.
    fn append_event(&mut self, event: SimulationEvent) -> Result<(), TracingError>;

    /// Return the event at zero-based `index`, or `None` if out of bounds.
    ///
    /// - `index`: Zero-based position in the trace log.
    fn get_event(&self, index: usize) -> Option<SimulationEvent>;

    /// Return a slice of all recorded events.
    ///
    /// For `VerificationBackend` this may return an empty slice or be unavailable;
    /// use `get_event` for individual access during formal verification.
    fn get_all_events(&self) -> &[SimulationEvent];

    /// Return the number of events recorded so far.
    fn event_count(&self) -> usize;

    /// Return the current global Lamport timestamp.
    ///
    /// Always equal to the maximum timestamp seen across all appended events.
    fn global_timestamp(&self) -> LamportTimestamp;

    /// Update the global timestamp if `ts` is greater than the current value.
    ///
    /// - `ts`: Candidate new global timestamp.
    ///
    /// Called by `TraceEngine` after each `append_event`.
    fn update_global_timestamp(&mut self, ts: LamportTimestamp);

    /// Clear all recorded events and reset the global timestamp, retaining storage capacity.
    fn clear(&mut self);

    /// Verify that the recorded trace satisfies causality invariants.
    ///
    /// Checks:
    /// - Per-thread timestamps are monotonically non-decreasing.
    /// - `global_timestamp()` equals the maximum event timestamp.
    ///
    /// Returns `Ok(())` if all invariants hold, or the first `TracingError` found.
    fn verify_causality(&self) -> Result<(), TracingError>;
}
