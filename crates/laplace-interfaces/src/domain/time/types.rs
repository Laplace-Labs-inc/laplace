//! Time system type definitions.
//!
//! Canonical types representing temporal state in the Laplace simulation engine.
//! Each type corresponds directly to a TLA+ state variable in `VirtualClock.tla`.

use crate::domain::memory::{Address, CoreId};
use std::cmp::Ordering;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Virtual time measured in nanoseconds.
///
/// Advances only when simulation events are processed (event-driven), not
/// continuously like wall-clock time.
///
/// # TLA+ Correspondence
/// `VARIABLE virtualTimeNs :: Nat`
pub type VirtualTimeNs = u64;

/// Lamport logical clock value for causality tracking.
///
/// Incremented on every event, providing a total ordering even when multiple
/// events share the same [`VirtualTimeNs`] timestamp.
///
/// # TLA+ Correspondence
/// `VARIABLE lamportClock :: Nat`
pub type LamportClock = u64;

/// Globally unique identifier for a scheduled event.
///
/// Assigned at scheduling time; used for tracing, debugging, and tie-breaking
/// when both time and Lamport clock collide.
pub type EventId = u64;

/// Mode that governs how virtual time advances.
///
/// Passed to clock constructors to select the advancement policy.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Time",
        link = "LEP-0011-laplace-interfaces-virtual_clock_model"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeMode {
    /// Advance time only when events are processed.
    ///
    /// Primary mode for deterministic simulation — no "empty time" intervals.
    EventDriven,

    /// Advance time continuously (reserved for future use).
    RealTime,
}

/// Payload carried by a scheduled simulation event.
///
/// Describes the action that fires when the event's scheduled time is reached.
/// Variants that touch memory use [`Address`] and [`CoreId`] from the memory domain.
///
/// # TLA+ Correspondence
/// ```tla
/// eventPayload \in {"Test", "MemoryWriteSync", "MemoryFence", "TaskReady", "WatchdogTimeout", "Custom"}
/// ```
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Time",
        link = "LEP-0011-laplace-interfaces-virtual_clock_model"
    )
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventPayload {
    /// Synthetic test event carrying an arbitrary numeric tag.
    ///
    /// Used in unit tests and verification harnesses.
    Test(u64),

    /// Signals that a buffered write should be flushed to main memory.
    MemoryWriteSync {
        /// Core that issued the write.
        core: CoreId,
        /// Memory address being written.
        addr: Address,
        /// Value to commit to main memory.
        value: u64,
    },

    /// Signals that a core's entire store buffer should be drained to main memory.
    MemoryFence {
        /// Core issuing the fence.
        core: CoreId,
    },

    /// Indicates a scheduled task is ready to execute.
    TaskReady {
        /// Unique task identifier.
        task_id: String,
    },

    /// Watchdog timer expiry for a specific tenant.
    WatchdogTimeout {
        /// Tenant whose watchdog timer expired.
        tenant_id: String,
    },

    /// Application-defined event with a string-encoded payload.
    Custom(String),
}

/// A single event in the priority-ordered event queue.
///
/// Events are ordered first by `scheduled_at_ns`, then by `lamport`, then by
/// `event_id` — all in ascending order (earlier = higher priority).
///
/// # TLA+ Correspondence
/// ```tla
/// event \in [time: VirtualTimeNs, lamport: LamportClock, id: EventId, payload: EventPayload]
/// ```
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Time",
        link = "LEP-0011-laplace-interfaces-virtual_clock_model"
    )
)]
#[derive(Debug, Clone)]
pub struct ScheduledEvent {
    /// Virtual time (ns) when this event should fire.
    pub scheduled_at_ns: VirtualTimeNs,

    /// Lamport clock value at scheduling time — secondary sort key.
    pub lamport: LamportClock,

    /// Globally unique event identifier — tertiary sort key.
    pub event_id: EventId,

    /// The action to execute when the event fires.
    pub payload: EventPayload,
}

impl ScheduledEvent {
    /// Create a new `ScheduledEvent`.
    ///
    /// - `scheduled_at_ns`: Virtual time when the event should fire.
    /// - `lamport`: Lamport clock value for causality ordering.
    /// - `event_id`: Unique event identifier.
    /// - `payload`: Action to execute when the event fires.
    ///
    /// Returns the constructed event.
    pub fn new(
        scheduled_at_ns: VirtualTimeNs,
        lamport: LamportClock,
        event_id: EventId,
        payload: EventPayload,
    ) -> Self {
        Self {
            scheduled_at_ns,
            lamport,
            event_id,
            payload,
        }
    }
}

/// Equality by event ID only — two events with the same ID are the same event.
impl PartialEq for ScheduledEvent {
    fn eq(&self, other: &Self) -> bool {
        self.event_id == other.event_id
    }
}

impl Eq for ScheduledEvent {}

impl PartialOrd for ScheduledEvent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Priority ordering for use in a max-heap (inverted so that earlier time = higher priority).
///
/// # TLA+ Correspondence
/// ```tla
/// SelectNextEvent ==
///     CHOOSE e \in eventQueue :
///         \A other \in eventQueue :
///             \/ e.time < other.time
///             \/ (e.time = other.time /\ e.lamport <= other.lamport)
/// ```
impl Ord for ScheduledEvent {
    fn cmp(&self, other: &Self) -> Ordering {
        match other.scheduled_at_ns.cmp(&self.scheduled_at_ns) {
            Ordering::Equal => match other.lamport.cmp(&self.lamport) {
                Ordering::Equal => other.event_id.cmp(&self.event_id),
                ord => ord,
            },
            ord => ord,
        }
    }
}
