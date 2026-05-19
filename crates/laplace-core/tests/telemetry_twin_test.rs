// SPDX-License-Identifier: Apache-2.0
#![cfg(feature = "twin")]
//! Telemetry Axiom Integration Tests — Sprint 3 (S-TEL1 ~ S-TEL6)
//!
//! Validates concurrent correctness, overflow safety, ring-buffer eviction,
//! snapshot integrity, singleton initialization, and reset-under-load for the
//! `GlobalTelemetry` / `EngineMetrics` / `EventRingBuffer` stack.
//!
//! # Notes on Global State
//!
//! `GlobalTelemetry` is a **process-wide static singleton**.  Tests that need
//! isolation create local `EngineMetrics` / `EventRingBuffer` instances via
//! `Arc::new(...)`.  Only `telemetry_singleton_init_race` intentionally exercises
//! the global singleton, which is the point of that test.
//!
//! | ID     | Name                                    | Verifies |
//! |--------|-----------------------------------------|----------|
//! | S-TEL1 | `telemetry_concurrent_increment_correctness` | 16 tasks × 1 000 incs == 16 000 (no loss) |
//! | S-TEL2 | `telemetry_no_counter_overflow_under_saturation` | 100 000 incs never panic; value monotonically non-decreasing |
//! | S-TEL3 | `telemetry_event_ring_buffer_capacity_eviction` | Full buffer evicts oldest on overflow |
//! | S-TEL4 | `telemetry_snapshot_consistency` | Concurrent push + snapshot never produces torn data |
//! | S-TEL5 | `telemetry_singleton_init_race` | 32 concurrent first-callers all receive the same instance |
//! | S-TEL6 | `telemetry_metrics_reset_under_load` | reset() under concurrent writes never panics |

use std::sync::Arc;

use laplace_core::domain::entropy::seed::ContextId;
use laplace_core::domain::telemetry::{
    EngineMetrics, EventRingBuffer, GlobalTelemetry, TelemetryEvent,
};

// ── S-TEL1 ────────────────────────────────────────────────────────────────────

/// 16 tokio tasks each call `inc_requests()` 1 000 times.
/// The final counter must equal exactly 16 000 — no increments lost.
///
/// # Correctness Guarantee
///
/// `EngineMetrics::inc_requests()` uses `AtomicU64::fetch_add(1, Relaxed)`.
/// Every fetch_add is a single indivisible RMW; no increment can be lost
/// regardless of scheduling order.  Joining all tasks before reading the
/// counter provides the necessary acquire/release barrier (via tokio's task
/// completion mechanism) so the main task sees all 16 000 writes.
#[tokio::test]
async fn telemetry_concurrent_increment_correctness() {
    let m = Arc::new(EngineMetrics::new());

    let mut handles = Vec::new();
    for _ in 0..16 {
        let m2 = Arc::clone(&m);
        handles.push(tokio::spawn(async move {
            for _ in 0..1_000 {
                m2.inc_requests();
            }
        }));
    }

    for handle in handles {
        handle.await.expect("task must not panic");
    }

    assert_eq!(
        m.total_requests(),
        16_000,
        "All 16 × 1 000 increments must be reflected in the final counter"
    );
}

// ── S-TEL2 ────────────────────────────────────────────────────────────────────

/// 100 000 sequential `inc_requests()` calls must never panic, and each
/// intermediate read must be ≥ the previous read (monotonically non-decreasing).
///
/// # Overflow Safety
///
/// `u64::MAX` ≈ 1.8 × 10¹⁹, so 100 000 increments are nowhere near overflow.
/// The test demonstrates the absence of arithmetic panic and confirms that
/// `Ordering::Relaxed` is sufficient for single-threaded monotonic reads.
#[tokio::test]
async fn telemetry_no_counter_overflow_under_saturation() {
    let m = EngineMetrics::new();

    let mut prev = m.total_requests();
    for _ in 0..100_000_u64 {
        m.inc_requests();
        let current = m.total_requests();
        assert!(
            current >= prev,
            "Counter must be monotonically non-decreasing (prev={}, current={})",
            prev,
            current
        );
        prev = current;
    }

    assert_eq!(
        m.total_requests(),
        100_000,
        "Final counter must equal exactly 100 000"
    );
}

// ── S-TEL3 ────────────────────────────────────────────────────────────────────

/// Fill the ring buffer to capacity (1 024), then push one additional event.
/// Verify:
/// 1. The buffer length remains ≤ capacity (no silent growth).
/// 2. The oldest event ("event-0") has been evicted.
/// 3. The newest event ("newest") is present at the tail.
/// 4. "event-1" is now the oldest (at index 0).
///
/// # Eviction Policy
///
/// `EventRingBuffer` uses `pop_front` before `push_back` when full — the
/// oldest element (front of the `VecDeque`) is always the one dropped.
#[tokio::test]
async fn telemetry_event_ring_buffer_capacity_eviction() {
    const CAPACITY: usize = 1_024;
    let buf = Arc::new(EventRingBuffer::new(CAPACITY));

    // Fill the buffer to exactly capacity with sequenced events
    for i in 0..CAPACITY {
        buf.push(TelemetryEvent::LogError(format!("event-{}", i)));
    }
    assert_eq!(buf.len(), CAPACITY, "Buffer must be at full capacity");

    // Overflow by one — "event-0" must be evicted
    buf.push(TelemetryEvent::LogError("newest".to_string()));

    let snap = buf.snapshot();

    // Length must remain at capacity (no unbounded growth)
    assert_eq!(
        snap.len(),
        CAPACITY,
        "Buffer length must remain at capacity after eviction"
    );

    // "event-0" (the oldest) must be gone
    let has_evicted = snap
        .iter()
        .any(|e| matches!(e, TelemetryEvent::LogError(msg) if msg == "event-0"));
    assert!(
        !has_evicted,
        "Oldest event (event-0) must have been evicted"
    );

    // "newest" (the just-pushed event) must be at the tail
    let has_newest = snap
        .iter()
        .any(|e| matches!(e, TelemetryEvent::LogError(msg) if msg == "newest"));
    assert!(has_newest, "Newest event must be present in the buffer");

    // "event-1" must now be at index 0 (the new oldest)
    match &snap[0] {
        TelemetryEvent::LogError(msg) => {
            assert_eq!(
                msg, "event-1",
                "After eviction of event-0, event-1 must be the new oldest"
            );
        }
        other => panic!("Expected LogError at index 0, got {:?}", other),
    }
}

// ── S-TEL4 ────────────────────────────────────────────────────────────────────

/// A writer task pushes 500 events while a reader task concurrently calls
/// `snapshot()` 100 times.  Each snapshot must be internally consistent:
/// - Length ≤ capacity (no torn growth beyond the ring limit).
/// - The returned `Vec` is an independent clone (reader never sees partial state).
///
/// # No Torn Reads
///
/// `EventRingBuffer::snapshot()` holds a `parking_lot::RwLock` read lock for
/// the entire duration of the clone.  `push()` takes a write lock.  By the
/// exclusion property of the lock, no snapshot can observe a partially-written
/// deque — each call atomically captures a complete point-in-time view.
#[tokio::test]
async fn telemetry_snapshot_consistency() {
    const CAPACITY: usize = 1_024;
    let buf = Arc::new(EventRingBuffer::new(CAPACITY));

    let buf_writer = Arc::clone(&buf);
    let buf_reader = Arc::clone(&buf);

    // Writer: push 500 events
    let writer = tokio::spawn(async move {
        for i in 0..500_u64 {
            buf_writer.push(TelemetryEvent::StateChanged(
                ContextId::new(i),
                format!("state-{}", i),
            ));
        }
    });

    // Reader: snapshot 100 times concurrently with the writer
    let reader = tokio::spawn(async move {
        for _ in 0..100 {
            let snap = buf_reader.snapshot();
            // A snapshot must never exceed the buffer's capacity
            assert!(
                snap.len() <= CAPACITY,
                "Snapshot length {} must not exceed capacity {}",
                snap.len(),
                CAPACITY
            );
            // Every element in the snapshot must be a valid, fully-formed event
            for event in &snap {
                if let TelemetryEvent::StateChanged(ctx, state) = event {
                    // Both fields must be coherent (non-empty state name)
                    let _ = ctx.as_u64();
                    assert!(
                        !state.is_empty(),
                        "State name must not be empty (torn read)"
                    );
                }
            }
        }
    });

    writer.await.expect("writer task must not panic");
    reader.await.expect("reader task must not panic");

    // Buffer must be in a valid final state
    let final_snap = buf.snapshot();
    assert!(
        final_snap.len() <= CAPACITY,
        "Final snapshot must be consistent"
    );
}

// ── S-TEL5 ────────────────────────────────────────────────────────────────────

/// 32 tokio tasks simultaneously call `GlobalTelemetry::metrics()`.
/// Every task must receive the **same static pointer** — the `OnceLock`
/// singleton must initialise exactly once regardless of which task "wins".
///
/// # Singleton Guarantee
///
/// `OnceLock::get_or_init` is documented to be safe for concurrent callers.
/// Only one closure executes; all other callers block (or spin briefly) until
/// the value is ready, then return the already-initialised reference.  The
/// resulting `&'static` pointer is the same for every caller.
#[tokio::test]
async fn telemetry_singleton_init_race() {
    let mut handles = Vec::new();

    // 32 tasks race to access the global singleton
    for _ in 0..32 {
        handles.push(tokio::spawn(async {
            let m = GlobalTelemetry::metrics();
            // Return the raw address of the static reference for comparison
            m as *const EngineMetrics as usize
        }));
    }

    let mut pointers = Vec::with_capacity(32);
    for handle in handles {
        pointers.push(handle.await.expect("task must not panic"));
    }

    // All 32 tasks must have received the exact same singleton address
    let first_ptr = pointers[0];
    for (i, &ptr) in pointers.iter().enumerate() {
        assert_eq!(
            ptr, first_ptr,
            "Task {} received a different pointer (0x{:x}) than task 0 (0x{:x})",
            i, ptr, first_ptr
        );
    }
}

// ── S-TEL6 ────────────────────────────────────────────────────────────────────

/// 8 writer tasks simultaneously call `inc_requests()` / `inc_active_vus()`
/// while a concurrent reset task calls `reset()` ten times.  The test verifies
/// that no data race, undefined behaviour, or panic occurs under this load.
///
/// # Relaxed-Ordering Contract
///
/// `EngineMetrics::reset()` uses `Ordering::Relaxed` and its doc-comment
/// explicitly notes that callers are responsible for ordering if strict
/// consistency is required.  This test intentionally exercises the *race*
/// scenario to verify that the atomic operations themselves never produce
/// undefined behaviour, even when reads and resets interleave arbitrarily.
///
/// # What We Assert
///
/// - No panic in any task (implicit: if the test completes, no panic occurred).
/// - Final counter reads are valid `u64` values (not corrupted by torn writes).
/// - `total_requests` and `active_vus` are independently readable after the race.
#[tokio::test]
async fn telemetry_metrics_reset_under_load() {
    const WRITERS: usize = 8;
    const INCS_PER_WRITER: u64 = 1_000;

    let m = Arc::new(EngineMetrics::new());

    // Spawn 8 writer tasks
    let mut writer_handles = Vec::new();
    for _ in 0..WRITERS {
        let m2 = Arc::clone(&m);
        writer_handles.push(tokio::spawn(async move {
            for _ in 0..INCS_PER_WRITER {
                m2.inc_requests();
                m2.inc_active_vus();
            }
        }));
    }

    // Concurrently reset 10 times on a dedicated task
    let m_reset = Arc::clone(&m);
    let reset_handle = tokio::spawn(async move {
        for _ in 0..10 {
            m_reset.reset();
            // Yield to allow writers to interleave with resets
            tokio::task::yield_now().await;
        }
    });

    // Await all tasks — no panic means no undefined behaviour
    for handle in writer_handles {
        handle.await.expect("writer task must not panic");
    }
    reset_handle.await.expect("reset task must not panic");

    // After all tasks complete, both counters must be valid readable u64 values.
    // The exact final value is indeterminate (any reset may have run last),
    // but it must be in [0, WRITERS * INCS_PER_WRITER] and must not panic.
    let final_requests = m.total_requests();
    let final_active_vus = m.active_vus();

    let max_possible = (WRITERS as u64) * INCS_PER_WRITER;
    assert!(
        final_requests <= max_possible,
        "final total_requests ({}) must not exceed maximum possible ({})",
        final_requests,
        max_possible
    );
    assert!(
        final_active_vus <= max_possible,
        "final active_vus ({}) must not exceed maximum possible ({})",
        final_active_vus,
        max_possible
    );
}
