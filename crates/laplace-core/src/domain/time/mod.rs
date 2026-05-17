// SPDX-License-Identifier: Apache-2.0
//! Time Abstraction Layer with TLA+ Formal Verification
//!
//! This module provides a unified interface for time sources that enables seamless
//! switching between production system time and virtual time for testing and
//! deterministic simulation. The abstraction uses static dispatch through generic
//! specialization to eliminate runtime overhead while maintaining flexibility.
//!
//! # TLA+ Specification Mapping
//!
//! This implementation corresponds to the formal specification in `specs/tla/VirtualClock.tla`:
//!
//! ```tla
//! VARIABLES virtualTimeNs, lamportClock, eventQueue
//!
//! Init ==
//!     /\ virtualTimeNs = 0
//!     /\ lamportClock = 0
//!     /\ eventQueue = {}
//! ```
//!
//! The Clock trait and ClockBackend trait, along with their implementations, provide concrete
//! Rust semantics for the abstract TLA+ model, with VirtualClock<B: ClockBackend> ensuring
//! that all state transitions maintain the TLA+ invariants.
//!
//! # Architecture
//!
//! The time module consists of three layers:
//!
//! **Clock Trait**: A simple abstraction for reading time in different units
//! (nanoseconds, microseconds, milliseconds). Implemented by both SystemClock
//! (production) and VirtualClock (verification).
//!
//! **Types** (`types.rs`): Core data types (VirtualTimeNs, LamportClock, EventPayload,
//! ScheduledEvent) that correspond to TLA+ variables and define event semantics.
//!
//! **ClockBackend Trait** (`backend.rs`): The primary abstraction for managing time state,
//! event queues, and Lamport clocks. Enables zero-cost polymorphism through static
//! dispatch, allowing different implementations for production and verification.
//!
//! # Zero-Cost Guarantee
//!
//! The abstraction layer has no runtime cost in production because:
//! - VirtualClock code is compiled out unless specifically requested (feature = "twin")
//! - SystemClock is a zero-overhead wrapper around `std::time::Instant`
//! - The Clock trait enables inlining of time queries
//! - ClockBackend uses static dispatch (no vtable lookups)
//!
//! # Monotonicity Invariant
//!
//! Both clock implementations guarantee strict monotonicity: once a time value is
//! returned, all subsequent queries return greater or equal values. This invariant
//! is formally verified using Kani symbolic execution when the `kani` feature is enabled.

pub mod backend;
pub mod production;
pub mod types;

#[cfg(feature = "twin")]
pub mod verification;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

// Re-export types and traits at module level
pub use backend::ClockBackend;
pub use production::ProductionBackend;
pub use types::{EventId, EventPayload, LamportClock, ScheduledEvent, TimeMode, VirtualTimeNs};

// Verification backend only available with feature = "twin"
#[cfg(feature = "twin")]
pub use verification::VerificationBackend;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Clock Trait: Abstract Time Interface
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Abstraction for time source providers.
///
/// This trait enables the kernel to remain independent of time source implementation,
/// allowing production code to use system time while testing and verification use
/// virtual time with explicit control over advancement.
///
/// # Thread Safety
///
/// All implementations must be `Send + Sync` to allow safe sharing across threads
/// and async task boundaries without additional synchronization overhead at the
/// caller level.
pub trait Clock: Send + Sync + fmt::Debug {
    /// Get current time in nanoseconds.
    ///
    /// # Monotonicity Guarantee
    ///
    /// For any two calls `t1 = now_ns()` and `t2 = now_ns()` where the second call
    /// happens after the first in real time, the invariant `t2 >= t1` is guaranteed.
    ///
    /// # Returns
    ///
    /// Current time as u64 nanoseconds since the clock's reference epoch.
    fn now_ns(&self) -> u64;

    /// Get current time in microseconds.
    ///
    /// # Returns
    ///
    /// Current time as u64 microseconds (derived from nanoseconds).
    fn now_us(&self) -> u64 {
        self.now_ns() / 1_000
    }

    /// Get current time in milliseconds.
    ///
    /// # Returns
    ///
    /// Current time as i64 milliseconds (derived from nanoseconds).
    /// Returned as i64 for compatibility with domain utilities.
    fn now_ms(&self) -> i64 {
        (self.now_ns() / 1_000_000) as i64
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// SystemClock: Production Implementation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Production-grade system clock implementation.
///
/// This clock implementation provides monotonic elapsed time using `std::time::Instant`.
/// It is suitable for all production deployments and offers zero overhead compared to
/// direct `Instant` usage due to compiler inlining and optimization.
///
/// # Design Notes
///
/// The clock captures a reference `Instant` at creation and all subsequent time queries
/// return elapsed nanoseconds since that reference point. This approach ensures
/// monotonicity and avoids the need for locks or atomic operations in the common path.
///
/// # Kani Verification
///
/// In Kani formal verification mode, this clock is mocked to return a deterministic
/// value (0) instead of calling `std::time::Instant::now()`, which is not supported
/// by the symbolic execution environment.
#[derive(Debug, Clone)]
pub struct SystemClock {
    /// Reference point for elapsed time calculations (production only).
    ///
    /// In Kani mode, this field is not used and time is always 0.
    #[cfg(not(kani))]
    start: std::time::Instant,
}

impl SystemClock {
    /// Create a new system clock.
    ///
    /// # Returns
    ///
    /// New clock initialized to current system instant (production) or time 0 (Kani).
    pub fn new() -> Self {
        #[cfg(not(kani))]
        {
            Self {
                start: std::time::Instant::now(),
            }
        }
        #[cfg(kani)]
        {
            Self {}
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now_ns(&self) -> u64 {
        #[cfg(not(kani))]
        {
            self.start.elapsed().as_nanos() as u64
        }
        #[cfg(kani)]
        {
            // In Kani verification mode, return a deterministic time value
            // that doesn't require system calls. This allows formal verification
            // to proceed without depending on the unsupported clock_gettime syscall.
            0u64
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// VirtualClock: Formal Verification Implementation with Event Scheduling
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Virtual clock for deterministic testing and simulation with formal verification.
///
/// This clock implementation wraps a pluggable ClockBackend that manages virtual time,
/// Lamport clocks, and event queues. This enables test code and the Axiom verification
/// engine to manipulate time programmatically while maintaining TLA+ invariants.
///
/// # Type Parameters
///
/// * `B` - The clock backend implementation (production or verification)
///
/// # Composition
///
/// VirtualClock<B: ClockBackend> combines:
/// - A ClockBackend for managing time state and event queue
/// - An atomic counter for generating unique event IDs
///
/// This design allows different backends to be used without changing the VirtualClock
/// interface, supporting both production scenarios and formal verification.
///
/// # Feature Gating
///
/// This implementation is only fully compiled when running tests or when the `twin`
/// feature is enabled. However, the struct definition is always available to support
/// generic code that accepts ClockBackend implementors.
pub struct VirtualClock<B: ClockBackend> {
    /// Backend managing time state and event queue.
    backend: B,
    /// Atomic counter for generating unique event IDs (direct ownership, no Arc overhead).
    next_event_id: AtomicU64,
}

impl<B: ClockBackend> VirtualClock<B> {
    /// Create a new virtual clock with the given backend.
    ///
    /// # Arguments
    ///
    /// * `backend` - The clock backend implementation
    ///
    /// # Returns
    ///
    /// New virtual clock initialized with the backend's initial state.
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            next_event_id: AtomicU64::new(1),
        }
    }

    /// Get a reference to the underlying backend.
    ///
    /// # Returns
    ///
    /// Reference to the clock backend.
    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Get mutable access to the underlying backend.
    ///
    /// # Returns
    ///
    /// Mutable reference to the clock backend.
    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// Get the current virtual time in nanoseconds.
    ///
    /// # Returns
    ///
    /// Current virtual time in nanoseconds.
    pub fn current_time(&self) -> VirtualTimeNs {
        self.backend.current_time()
    }

    /// Get the current Lamport clock value.
    ///
    /// # Returns
    ///
    /// Current Lamport logical clock value.
    pub fn current_lamport(&self) -> LamportClock {
        self.backend.current_lamport()
    }

    /// Schedule a generic event with the given delay.
    ///
    /// # Arguments
    ///
    /// * `delay_ns` - Delay in nanoseconds before the event fires
    /// * `payload` - The event's payload (action to execute)
    ///
    /// # Returns
    ///
    /// `true` if the event was successfully scheduled, `false` if the queue is full.
    pub fn schedule(&mut self, delay_ns: VirtualTimeNs, payload: EventPayload) -> bool {
        let event_id = self.next_event_id.fetch_add(1, Ordering::SeqCst);
        let scheduled_at = self.backend.current_time() + delay_ns;
        let lamport = self.backend.current_lamport();

        let event = ScheduledEvent::new(scheduled_at, lamport, event_id, payload);
        self.backend.push_event(event)
    }

    /// Schedule a memory write synchronization event.
    ///
    /// This is a convenience method that creates a MemoryWriteSync event and
    /// schedules it with the given delay.
    ///
    /// # Arguments
    ///
    /// * `delay_ns` - Delay in nanoseconds before the write syncs
    /// * `core` - The core performing the write
    /// * `addr` - The memory address being written
    /// * `value` - The value to write
    ///
    /// # Returns
    ///
    /// `true` if the event was successfully scheduled, `false` if the queue is full.
    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(layer = "20_Core_Time", link = "LEP-0001-laplace-core-time_domain")
    )]
    pub fn schedule_write_sync(
        &mut self,
        delay_ns: VirtualTimeNs,
        core: crate::domain::memory::CoreId,
        addr: crate::domain::memory::Address,
        value: u64,
    ) -> bool {
        self.schedule(
            delay_ns,
            EventPayload::MemoryWriteSync { core, addr, value },
        )
    }

    /// Schedule a memory fence event.
    ///
    /// This is a convenience method that creates a MemoryFence event and
    /// schedules it with the given delay.
    ///
    /// # Arguments
    ///
    /// * `delay_ns` - Delay in nanoseconds before the fence executes
    /// * `core` - The core issuing the fence
    ///
    /// # Returns
    ///
    /// `true` if the event was successfully scheduled, `false` if the queue is full.
    pub fn schedule_fence(
        &mut self,
        delay_ns: VirtualTimeNs,
        core: crate::domain::memory::CoreId,
    ) -> bool {
        self.schedule(delay_ns, EventPayload::MemoryFence { core })
    }

    /// Get the next scheduled event without removing it.
    ///
    /// # Returns
    ///
    /// A reference to the next event, or None if the queue is empty.
    pub fn peek_next_event(&self) -> Option<()> {
        // Note: This would require backend to support peeking.
        // For now, this is a placeholder for future implementation.
        None
    }

    /// Process the next scheduled event.
    ///
    /// Removes and returns the next event from the queue, advancing virtual time
    /// to the event's scheduled time and updating the Lamport clock.
    ///
    /// # Returns
    ///
    /// The next scheduled event, or None if the queue is empty.
    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(layer = "20_Core_Time", link = "LEP-0001-laplace-core-time_domain")
    )]
    pub fn tick(&mut self) -> Option<ScheduledEvent> {
        let event = self.backend.pop_event()?;

        // Advance virtual time to the event's scheduled time
        self.backend.set_time(event.scheduled_at_ns);

        // Increment Lamport clock for causality tracking
        self.backend.increment_lamport();

        Some(event)
    }

    /// Check if the event queue is empty.
    ///
    /// # Returns
    ///
    /// `true` if there are no pending events, `false` otherwise.
    pub fn is_queue_empty(&self) -> bool {
        self.backend.is_empty()
    }

    /// Get the number of pending events in the queue.
    ///
    /// # Returns
    ///
    /// The number of events currently scheduled.
    pub fn queue_len(&self) -> usize {
        self.backend.queue_len()
    }

    /// Reset the entire time subsystem to its initial state.
    ///
    /// This clears all pending events, resets virtual time and the Lamport clock
    /// to their initial values.
    pub fn reset(&mut self) {
        self.backend.reset();
        self.next_event_id.store(1, Ordering::SeqCst);
    }
}

impl<B: ClockBackend + Send + Sync> Clock for VirtualClock<B> {
    fn now_ns(&self) -> u64 {
        self.backend.current_time()
    }
}

impl<B: ClockBackend> fmt::Debug for VirtualClock<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VirtualClock")
            .field("current_time", &self.backend.current_time())
            .field("current_lamport", &self.backend.current_lamport())
            .field("queue_len", &self.backend.queue_len())
            .finish()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Formal Verification Harnesses
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(all(kani, feature = "twin"))]
mod proofs;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Unit Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_clock_monotonic() {
        let clock = SystemClock::new();
        let t1 = clock.now_ns();
        let t2 = clock.now_ns();
        assert!(t2 >= t1, "System clock must be monotonic");
    }

    #[test]
    fn test_system_clock_units_consistent() {
        let clock = SystemClock::new();
        let ns = clock.now_ns();
        let us = clock.now_us();
        let ms = clock.now_ms();

        // Allow 1 unit tolerance for execution time between calls
        let us_expected_min = ns / 1_000;
        let us_expected_max = us_expected_min + 1;
        assert!(
            us >= us_expected_min && us <= us_expected_max,
            "us: {} out of range [{}, {}]",
            us,
            us_expected_min,
            us_expected_max
        );

        let ms_expected_min = ns / 1_000_000;
        let ms_expected_max = ms_expected_min + 1;
        assert!(
            (ms as u64) >= ms_expected_min && (ms as u64) <= ms_expected_max,
            "ms: {} out of range [{}, {}]",
            ms,
            ms_expected_min,
            ms_expected_max
        );
    }

    #[test]
    fn test_clock_trait_conversions() {
        let clock: &dyn Clock = &SystemClock::new();
        let ns = clock.now_ns();
        let us = clock.now_us();
        let ms = clock.now_ms();

        // Allow 1 unit tolerance for execution time between calls
        let us_expected_min = ns / 1_000;
        let us_expected_max = us_expected_min + 1;
        assert!(
            us >= us_expected_min && us <= us_expected_max,
            "us: {} out of range [{}, {}]",
            us,
            us_expected_min,
            us_expected_max
        );

        let ms_expected_min = ns / 1_000_000;
        let ms_expected_max = ms_expected_min + 1;
        assert!(
            (ms as u64) >= ms_expected_min && (ms as u64) <= ms_expected_max,
            "ms: {} out of range [{}, {}]",
            ms,
            ms_expected_min,
            ms_expected_max
        );
    }
}
