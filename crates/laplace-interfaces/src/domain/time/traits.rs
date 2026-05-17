//! Clock backend trait definition.
//!
//! Defines the abstract interface that all time-backend implementations must satisfy.
//! Consumers (e.g. `laplace-core`'s `VirtualClock`) program to this trait so that
//! production and verification backends are interchangeable at compile time with
//! zero runtime overhead.

use super::types::{LamportClock, ScheduledEvent, VirtualTimeNs};

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Abstract interface for clock state management.
///
/// Implement this trait to provide a concrete backend for clock operations.
/// The trait models three TLA+ state variables:
///
/// - **`virtualTimeNs`**: monotonic virtual time in nanoseconds.
/// - **`lamportClock`**: logical clock for causal ordering.
/// - **`eventQueue`**: priority-ordered queue of [`ScheduledEvent`]s.
///
/// # TLA+ Invariants
///
/// Implementations must maintain:
/// ```tla
/// Invariant ==
///     /\ virtualTimeNs >= 0
///     /\ lamportClock >= 0
///     /\ virtualTimeNs is monotonically non-decreasing
/// ```
///
/// # Implementing
///
/// Both `ProductionBackend` (heap-allocated, `BinaryHeap`) and `VerificationBackend`
/// (stack-allocated, fixed-size array) implement this trait in `laplace-core`.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Time",
        link = "LEP-0011-laplace-interfaces-virtual_clock_model"
    )
)]
pub trait ClockBackend {
    /// Return the current virtual time in nanoseconds.
    ///
    /// The returned value is always ≥ any previously returned value (monotonic).
    ///
    /// # TLA+ Correspondence
    /// `Read(time) == time = virtualTimeNs`
    fn current_time(&self) -> VirtualTimeNs;

    /// Return the current Lamport logical clock value.
    ///
    /// # TLA+ Correspondence
    /// `Read(lamport) == lamport = lamportClock`
    fn current_lamport(&self) -> LamportClock;

    /// Set virtual time to `time`.
    ///
    /// - `time`: New virtual time in nanoseconds.
    ///
    /// The caller must ensure `time >= current_time()` to preserve monotonicity.
    ///
    /// # TLA+ Correspondence
    /// `SetTime(newTime) == virtualTimeNs' = newTime`
    fn set_time(&mut self, time: VirtualTimeNs);

    /// Increment the Lamport clock and return the new value.
    ///
    /// Returns the incremented clock value, which should be assigned to any
    /// newly created event.
    ///
    /// # TLA+ Correspondence
    /// `IncrementLamport == lamportClock' = lamportClock + 1`
    fn increment_lamport(&mut self) -> LamportClock;

    /// Reset the Lamport clock to zero.
    ///
    /// Typically called when the event queue becomes empty to prevent unbounded growth.
    ///
    /// # TLA+ Correspondence
    /// `ResetLamport == IF eventQueue' = {} THEN lamportClock' = 0 ELSE lamportClock' unchanged`
    fn reset_lamport(&mut self);

    /// Enqueue a [`ScheduledEvent`].
    ///
    /// - `event`: The event to add.
    ///
    /// Returns `true` on success, `false` if the queue is full (bounded backends only).
    ///
    /// # TLA+ Correspondence
    /// `PushEvent(event) == eventQueue' = eventQueue \cup {event}`
    fn push_event(&mut self, event: ScheduledEvent) -> bool;

    /// Dequeue and return the highest-priority event (earliest time, then lowest Lamport).
    ///
    /// Returns `Some(event)` if the queue is non-empty, `None` otherwise.
    ///
    /// # TLA+ Correspondence
    /// `PopEvent == CHOOSE e \in eventQueue : \A other \in eventQueue : e.time <= other.time`
    fn pop_event(&mut self) -> Option<ScheduledEvent>;

    /// Return `true` if the event queue contains no events.
    fn is_empty(&self) -> bool;

    /// Return the number of events currently in the queue.
    fn queue_len(&self) -> usize;

    /// Reset all time state (virtual time, Lamport clock, event queue) to zero/empty.
    ///
    /// # TLA+ Correspondence
    /// ```tla
    /// Reset ==
    ///     /\ virtualTimeNs' = 0
    ///     /\ lamportClock' = 0
    ///     /\ eventQueue' = {}
    /// ```
    fn reset(&mut self);
}
