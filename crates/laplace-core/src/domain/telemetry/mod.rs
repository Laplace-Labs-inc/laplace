// SPDX-License-Identifier: Apache-2.0
//! Telemetry — Zero-Cost Observability Without Async Runtime
//!
//! Provides two complementary channels for feeding a TUI or monitoring system:
//!
//! | Channel | Type | Mechanism | When |
//! |---------|------|-----------|------|
//! | High-frequency counters | [`EngineMetrics`] | `AtomicU64` (lock-free) | Always |
//! | Discrete events | [`EventRingBuffer`] | `parking_lot::RwLock` + `VecDeque` | `feature = "verification"` |
//!
//! Access both through the [`GlobalTelemetry`] singleton:
//!
//! ```rust,ignore
//! use laplace_core::domain::telemetry::{GlobalTelemetry, TelemetryEvent};
//!
//! // Hot-path counter update (zero overhead)
//! GlobalTelemetry::metrics().inc_requests();
//!
//! // Discrete event (requires feature = "verification")
//! GlobalTelemetry::events().push(TelemetryEvent::LogError("oh no".to_string()));
//! ```
//!
//! # Architectural Guarantees
//!
//! - **No `tokio`**: all synchronisation is `std` + `parking_lot`.
//! - **No heap allocation at startup**: `OnceLock` defers initialisation to
//!   first access; nothing is allocated until the singleton is first touched.
//! - **No new external dependencies**: `parking_lot` is already part of the
//!   `verification` feature tier; `OnceLock` is in `std` since Rust 1.70.

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Sub-modules
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub mod metrics;

/// Discrete event ring buffer — requires `feature = "verification"` (parking_lot).
#[cfg(feature = "verification")]
pub mod events;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Public Re-exports
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub use metrics::EngineMetrics;
pub use metrics::VuMetricEvent;

#[cfg(feature = "twin")]
pub use metrics::MetricCollector;

#[cfg(feature = "verification")]
pub use events::{EventRingBuffer, TelemetryEvent};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Singleton state (private)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use std::sync::OnceLock;

/// Internal state held by the global telemetry singleton.
///
/// Fields are `#[cfg]`-gated so the struct is always valid regardless of which
/// features are active.
struct TelemetryState {
    metrics: EngineMetrics,

    #[cfg(feature = "verification")]
    events: EventRingBuffer,
}

/// Lazily-initialised global telemetry state.
///
/// `OnceLock` ensures thread-safe, one-time initialisation with no locking
/// overhead on subsequent reads (after the first access, it's a plain pointer
/// dereference).
static GLOBAL_TELEMETRY: OnceLock<TelemetryState> = OnceLock::new();

/// Return a reference to the singleton `TelemetryState`, initialising it on
/// the first call.
#[inline]
fn state() -> &'static TelemetryState {
    GLOBAL_TELEMETRY.get_or_init(|| TelemetryState {
        metrics: EngineMetrics::new(),

        #[cfg(feature = "verification")]
        events: EventRingBuffer::new(1024),
    })
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// GlobalTelemetry
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Namespace for the process-wide telemetry singleton.
///
/// `GlobalTelemetry` is a zero-size type — it exists solely to group the
/// `metrics()` and `events()` accessor methods. There is no instance to create.
///
/// # Thread Safety
///
/// All accessors return `&'static` references that are `Send + Sync`. The
/// underlying `OnceLock` guarantees data-race-free initialisation.
///
/// # Example
///
/// ```rust,ignore
/// use laplace_core::domain::telemetry::GlobalTelemetry;
///
/// GlobalTelemetry::metrics().inc_requests();
/// assert!(GlobalTelemetry::metrics().total_requests() >= 1);
/// ```
pub struct GlobalTelemetry;

impl GlobalTelemetry {
    /// Access the lock-free [`EngineMetrics`] singleton.
    ///
    /// Always available — no feature gate required.
    ///
    /// # Usage
    ///
    /// ```rust,ignore
    /// GlobalTelemetry::metrics().inc_requests();
    /// GlobalTelemetry::metrics().inc_active_vus();
    /// let count = GlobalTelemetry::metrics().total_requests();
    /// ```
    #[inline]
    pub fn metrics() -> &'static EngineMetrics {
        &state().metrics
    }

    /// Access the [`EventRingBuffer`] singleton.
    ///
    /// Requires `feature = "verification"` (uses `parking_lot::RwLock`).
    ///
    /// # Usage
    ///
    /// ```rust,ignore
    /// use laplace_core::domain::telemetry::{GlobalTelemetry, TelemetryEvent};
    ///
    /// GlobalTelemetry::events().push(TelemetryEvent::LogError("err".to_string()));
    /// let snap = GlobalTelemetry::events().snapshot();
    /// ```
    #[cfg(feature = "verification")]
    #[inline]
    pub fn events() -> &'static EventRingBuffer {
        &state().events
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_singleton_accessible() {
        let m = GlobalTelemetry::metrics();
        m.inc_requests();
        // The counter may already have been incremented by previous tests
        // running in the same process (shared static). We only verify it
        // is reachable and non-panicking.
        let _ = m.total_requests();
    }

    #[cfg(feature = "verification")]
    #[test]
    fn test_events_singleton_accessible() {
        use crate::domain::entropy::seed::ContextId;

        let ev = GlobalTelemetry::events();
        ev.push(TelemetryEvent::LogError("integration-test".to_string()));
        ev.push(TelemetryEvent::DporBacktrack(ContextId::new(99)));

        let snap = ev.snapshot();
        // Snapshot has at least our two events (may have more from other tests)
        assert!(snap.len() >= 2);
    }

    #[cfg(feature = "verification")]
    #[test]
    fn test_global_telemetry_call_pattern() {
        use crate::domain::entropy::seed::ContextId;

        // Verify the intended ergonomic call pattern compiles and works.
        GlobalTelemetry::metrics().inc_requests();
        GlobalTelemetry::events().push(TelemetryEvent::StateChanged(
            ContextId::new(1),
            "Thinking".to_string(),
        ));

        // Both accessors return &'static and can be called in any order.
        let _req = GlobalTelemetry::metrics().total_requests();
        let _snap = GlobalTelemetry::events().snapshot();
    }
}
