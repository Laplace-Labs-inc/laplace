// SPDX-License-Identifier: Apache-2.0
//! Discrete Event Ring Buffer
//!
//! Provides a bounded, thread-safe queue for low-frequency telemetry events.
//! The TUI drains the buffer via [`EventRingBuffer::snapshot`] at its own pace.
//!
//! # Design Invariants
//!
//! - **Lock-free push**: `crossbeam_queue::SegQueue` allows any number of
//!   producers to push without contention.
//! - **Bounded memory**: once the buffer reaches `capacity`, the oldest event
//!   is discarded before inserting the new one.
//! - **Serialized snapshot**: `snapshot()` holds an internal mutex so that
//!   concurrent readers cannot interleave drain/repush steps and lose events.
//! - **Clone-on-read**: `snapshot()` returns a `Vec<TelemetryEvent>` so the
//!   TUI holds its own copy without blocking writers.

use crossbeam_queue::SegQueue;
use std::sync::Mutex;

use crate::domain::entropy::seed::ContextId;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TelemetryEvent
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// A discrete telemetry event emitted by the simulation engine.
///
/// Events are low-frequency and carry richer context than atomic counters.
/// They are stored in [`EventRingBuffer`] and consumed by the TUI renderer.
///
/// # Variants
///
/// - [`LogError`]: a human-readable error message (structured logging).
/// - [`DporBacktrack`]: DPOR backtracked from the given context's execution path.
/// - [`StateChanged`]: a context transitioned to a new named state.
/// - [`ApiTrace`]: a captured API request/response trace (Phase 2.3).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum TelemetryEvent {
    /// A log-level error message from any subsystem.
    ///
    /// # Example
    /// ```rust,ignore
    /// GlobalTelemetry::events().push(TelemetryEvent::LogError(
    ///     "seed distributor: quota exceeded".to_string()
    /// ));
    /// ```
    LogError(String),

    /// The DPOR scheduler backtracked at the given context.
    ///
    /// Emitted when Ki-DPOR or Classic-DPOR resets exploration to a
    /// previously committed state, allowing the TUI to visualise search
    /// branching in real time.
    DporBacktrack(ContextId),

    /// A context changed its operational state.
    ///
    /// # Fields
    /// - `ContextId`: the context (VU / thread) that transitioned.
    /// - `String`:    the new state name (e.g. `"Thinking"`, `"Requesting"`).
    StateChanged(ContextId, String),

    /// Phase 2.3: API request/response trace captured by the executor.
    ///
    /// # Fields
    /// - `method`: HTTP method (e.g. `"GET"`, `"POST"`).
    /// - `path`:   Request URL path (e.g. `"/api/users"`).
    /// - `payload`: Serialised request/response body summary.
    /// - `is_error`: `true` if the response indicated a failure (4xx/5xx).
    ApiTrace {
        method: String,
        path: String,
        payload: String,
        is_error: bool,
    },

    /// Axiom Oracle detected a liveness violation (deadlock / starvation / invariant).
    ///
    /// The payload is the `Debug`-formatted `LivenessViolation` string,
    /// e.g. `"Deadlock { cycle: [ThreadId(0), ThreadId(1)] }"`.
    AxiomViolation(String),

    /// Axiom Oracle completed exhaustive DPOR search without any violation.
    AxiomClean,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// EventRingBuffer
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Bounded, thread-safe ring buffer for [`TelemetryEvent`] values.
///
/// Uses a `parking_lot::RwLock<VecDeque<TelemetryEvent>>` internally:
/// - **Writers** (`push`) take an exclusive write lock for the minimum time
///   needed to `pop_front` (if full) and `push_back`.
/// - **Readers** (`snapshot`, `len`) take a shared read lock; multiple TUI
///   threads can snapshot concurrently without blocking writers for long.
///
/// # Overflow Policy
///
/// When the buffer is full, the **oldest** event (front of the deque) is
/// silently dropped before the new event is appended. This is appropriate
/// for live dashboards where recent events are more relevant.
///
/// # Example
///
/// ```rust,ignore
/// let buf = EventRingBuffer::new(4);
/// buf.push(TelemetryEvent::LogError("boot".to_string()));
/// let events = buf.snapshot(); // Vec with one element
/// assert_eq!(events.len(), 1);
/// ```
pub struct EventRingBuffer {
    capacity: usize,
    buffer: SegQueue<TelemetryEvent>,
    /// Serializes concurrent `snapshot()` callers so the drain/repush cycle
    /// is never interleaved between two readers.
    snapshot_mutex: Mutex<()>,
}

impl EventRingBuffer {
    /// Create a new ring buffer with the given `capacity`.
    ///
    /// Pre-allocates the deque to avoid reallocation up to `capacity` events.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            buffer: SegQueue::new(),
            snapshot_mutex: Mutex::new(()),
        }
    }

    /// Push a new event into the buffer.
    ///
    /// If the buffer has reached `capacity`, the oldest event (front) is
    /// dropped to make room before the new event is appended to the back.
    pub fn push(&self, event: TelemetryEvent) {
        if self.capacity == 0 {
            return;
        }
        self.buffer.push(event);
        while self.buffer.len() > self.capacity {
            let _ = self.buffer.pop();
        }
    }

    /// Return a point-in-time snapshot of all buffered events.
    ///
    /// Events are returned in order from oldest (index 0) to newest (last).
    /// The returned `Vec` is an independent clone — the buffer is not modified.
    ///
    /// # Concurrency
    ///
    /// Concurrent `push()` callers remain lock-free and are never blocked.
    /// Concurrent `snapshot()` callers are serialized via `snapshot_mutex` so
    /// that the internal drain/repush cycle cannot interleave between two readers
    /// (which would otherwise cause event loss or duplication).
    ///
    /// Events pushed by a concurrent `push()` while this method holds the drain
    /// window will appear in the **next** `snapshot()` call.
    pub fn snapshot(&self) -> Vec<TelemetryEvent> {
        let _guard = self
            .snapshot_mutex
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut events = Vec::with_capacity(self.buffer.len());
        while let Some(event) = self.buffer.pop() {
            events.push(event);
        }
        for event in events.iter().cloned() {
            self.buffer.push(event);
        }
        events
    }

    /// Current number of events in the buffer.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Returns `true` if the buffer contains no events.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Maximum number of events the buffer can hold before eviction begins.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Remove all events from the buffer.
    ///
    /// Useful for test teardown or scenario resets.
    pub fn clear(&self) {
        while self.buffer.pop().is_some() {}
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_and_snapshot() {
        let buf = EventRingBuffer::new(8);
        buf.push(TelemetryEvent::LogError("boot".to_string()));
        buf.push(TelemetryEvent::DporBacktrack(ContextId::new(1)));

        let snap = buf.snapshot();
        assert_eq!(snap.len(), 2);
        assert!(matches!(snap[0], TelemetryEvent::LogError(_)));
        assert!(matches!(
            snap[1],
            TelemetryEvent::DporBacktrack(ContextId(1))
        ));
    }

    #[test]
    fn test_ring_evicts_oldest_when_full() {
        let buf = EventRingBuffer::new(3);
        buf.push(TelemetryEvent::LogError("first".to_string()));
        buf.push(TelemetryEvent::LogError("second".to_string()));
        buf.push(TelemetryEvent::LogError("third".to_string()));
        // Buffer is now full (capacity = 3)
        buf.push(TelemetryEvent::LogError("fourth".to_string()));

        let snap = buf.snapshot();
        assert_eq!(snap.len(), 3);

        // "first" was evicted; oldest is now "second"
        if let TelemetryEvent::LogError(msg) = &snap[0] {
            assert_eq!(msg, "second");
        } else {
            panic!("Expected LogError");
        }
        if let TelemetryEvent::LogError(msg) = &snap[2] {
            assert_eq!(msg, "fourth");
        } else {
            panic!("Expected LogError");
        }
    }

    #[test]
    fn test_len_and_is_empty() {
        let buf = EventRingBuffer::new(4);
        assert!(buf.is_empty());
        buf.push(TelemetryEvent::LogError("x".to_string()));
        assert_eq!(buf.len(), 1);
        assert!(!buf.is_empty());
    }

    #[test]
    fn test_capacity() {
        let buf = EventRingBuffer::new(1024);
        assert_eq!(buf.capacity(), 1024);
    }

    #[test]
    fn test_clear() {
        let buf = EventRingBuffer::new(4);
        buf.push(TelemetryEvent::LogError("y".to_string()));
        buf.clear();
        assert!(buf.is_empty());
    }

    #[test]
    fn test_state_changed_event() {
        let buf = EventRingBuffer::new(4);
        buf.push(TelemetryEvent::StateChanged(
            ContextId::new(42),
            "Requesting".to_string(),
        ));
        let snap = buf.snapshot();
        assert_eq!(snap.len(), 1);
        if let TelemetryEvent::StateChanged(ctx, state) = &snap[0] {
            assert_eq!(ctx.as_u64(), 42);
            assert_eq!(state, "Requesting");
        } else {
            panic!("Expected StateChanged");
        }
    }

    #[test]
    fn test_telemetry_segqueue_mpsc_ordering() {
        let buf = std::sync::Arc::new(EventRingBuffer::new(64));
        let mut handles = Vec::new();

        for worker in 0..4 {
            let buf = std::sync::Arc::clone(&buf);
            handles.push(std::thread::spawn(move || {
                for seq in 0..8 {
                    buf.push(TelemetryEvent::LogError(format!("{worker}:{seq}")));
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let snap = buf.snapshot();
        assert_eq!(snap.len(), 32);
        assert_eq!(buf.len(), 32);
    }

    #[test]
    fn test_snapshot_does_not_clear_buffer() {
        let buf = EventRingBuffer::new(4);
        buf.push(TelemetryEvent::LogError("persist".to_string()));
        let _ = buf.snapshot();
        assert_eq!(buf.len(), 1); // buffer unchanged after snapshot
    }
}
