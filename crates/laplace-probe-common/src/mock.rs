// SPDX-License-Identifier: Apache-2.0
//! MockProbeSource: deterministic synthetic `RawProbeEvent` generator.
//!
//! Produces realistic event sequences that simulate the output of the eBPF kernel
//! agent, covering all four concurrency bug patterns detectable by the Axiom DPOR
//! engine. All scenarios are deterministic and reproducible given the same seed.
//!
//! # Generated scenarios
//!
//! | Method                         | Bug pattern                              |
//! |--------------------------------|------------------------------------------|
//! | [`generate_ab_ba_deadlock`]    | AB-BA circular-wait deadlock             |
//! | [`generate_data_race`]         | Two threads on same resource without sync|
//! | [`generate_livelock_starvation`] | Starvation (counter > MAX threshold)   |
//!
//! [`generate_ab_ba_deadlock`]: MockProbeSource::generate_ab_ba_deadlock
//! [`generate_data_race`]: MockProbeSource::generate_data_race
//! [`generate_livelock_starvation`]: MockProbeSource::generate_livelock_starvation

use crate::{ProbeEventType, RawProbeEvent};

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

// ─────────────────────────────────────────────────────────────────────────────
// Constants — realistic-looking kernel values
// ─────────────────────────────────────────────────────────────────────────────

/// Kernel TID for the first mock thread.
pub const MOCK_TID_A: u32 = 1_001;
/// Kernel TID for the second mock thread.
pub const MOCK_TID_B: u32 = 1_002;
/// Kernel TID for a third mock thread (used in livelock scenario).
pub const MOCK_TID_C: u32 = 1_003;

/// Virtual mutex address for lock X (used in AB-BA and data-race scenarios).
pub const MOCK_LOCK_X: u64 = 0xFFFF_DEAD_BEEF_0000;
/// Virtual mutex address for lock Y (used in AB-BA scenario).
pub const MOCK_LOCK_Y: u64 = 0xFFFF_DEAD_BEEF_0008;

/// Simulated base clock (1 second in nanoseconds).
const BASE_CLOCK_NS: u64 = 1_000_000_000;

// ─────────────────────────────────────────────────────────────────────────────
// MockProbeSource
// ─────────────────────────────────────────────────────────────────────────────

/// Deterministic generator for synthetic `RawProbeEvent` sequences.
///
/// All timestamps are monotonically increasing, starting from [`BASE_CLOCK_NS`]
/// and advancing by 100 ns per event. This ensures correct ordering semantics
/// through the decoder and adapter layers.
///
/// # Example
///
/// ```
/// use laplace_probe_common::mock::MockProbeSource;
///
/// let events = MockProbeSource::new(42).generate_ab_ba_deadlock();
/// assert!(!events.is_empty());
/// ```
#[derive(Debug)]
pub struct MockProbeSource {
    /// Deterministic seed (currently used as the monotonic clock offset).
    seed: u64,
}

impl MockProbeSource {
    /// Creates a new source with the given seed.
    pub fn new(seed: u64) -> Self {
        Self { seed }
    }

    /// Creates a source with a default seed (`0xCAFE_BABE`).
    pub fn default_seed() -> Self {
        Self::new(0xCAFE_BABE)
    }

    // ── Scenario generators ────────────────────────────────────────────────

    /// Generates a clean, uncontested single-thread scenario.
    ///
    /// One thread acquires a single lock and immediately releases it with no
    /// other threads present. The Axiom Oracle must return `Clean`.
    ///
    /// Use this as a sanity check or to verify that the pipeline correctly
    /// distinguishes clean from buggy executions.
    pub fn generate_clean_uncontested(&self) -> Vec<RawProbeEvent> {
        let mut ts = BASE_CLOCK_NS + self.seed % 1_000_000;
        vec![
            make_thread_spawn(MOCK_TID_A, 0, {
                let t = ts;
                ts += 100;
                t
            }),
            make_lock_acquire(MOCK_TID_A, MOCK_LOCK_X, 0, {
                let t = ts;
                ts += 100;
                t
            }),
            make_lock_acquired(MOCK_TID_A, MOCK_LOCK_X, 0, {
                let t = ts;
                ts += 100;
                t
            }),
            make_lock_release(MOCK_TID_A, MOCK_LOCK_X, ts),
        ]
    }

    /// Generates the canonical AB-BA deadlock scenario.
    ///
    /// Thread A acquires lock X, then requests lock Y.
    /// Thread B acquires lock Y, then requests lock X.
    ///
    /// The DPOR engine will explore the interleaving where both threads hold one
    /// lock and block on the other, forming a circular wait → `Deadlock` verdict.
    ///
    /// # Event sequence (program-order, not time-order)
    ///
    /// ```text
    /// TID_A: ThreadSpawn → LockAcquire(X) → LockAcquired(X) → LockAcquire(Y) [blocks]
    /// TID_B: ThreadSpawn → LockAcquire(Y) → LockAcquired(Y) → LockAcquire(X) [blocks]
    /// ```
    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(
            layer = "40_Probe_Common",
            link = "LEP-0013-laplace-probe-common_compaction_and_sovereignty"
        )
    )]
    pub fn generate_ab_ba_deadlock(&self) -> Vec<RawProbeEvent> {
        let mut ts = BASE_CLOCK_NS + self.seed % 1_000_000;
        let mut events = Vec::with_capacity(12);

        // ── Thread spawns ──
        events.push(make_thread_spawn(MOCK_TID_A, 0, ts));
        ts += 100;
        events.push(make_thread_spawn(MOCK_TID_B, MOCK_TID_A, ts));
        ts += 100;

        // ── Thread A: acquire lock X (no contention) ──
        events.push(make_lock_acquire(MOCK_TID_A, MOCK_LOCK_X, 0, ts));
        ts += 50;
        events.push(make_lock_acquired(MOCK_TID_A, MOCK_LOCK_X, 0, ts));
        ts += 50;

        // ── Thread B: acquire lock Y (no contention) ──
        events.push(make_lock_acquire(MOCK_TID_B, MOCK_LOCK_Y, 0, ts));
        ts += 50;
        events.push(make_lock_acquired(MOCK_TID_B, MOCK_LOCK_Y, 0, ts));
        ts += 50;

        // ── Thread A: requests lock Y — will block (B holds it) ──
        events.push(make_lock_acquire(MOCK_TID_A, MOCK_LOCK_Y, 0, ts));
        ts += 100;
        events.push(make_lock_contention(MOCK_TID_A, MOCK_LOCK_Y, ts));
        ts += 100;

        // ── Thread B: requests lock X — will block (A holds it) ──
        events.push(make_lock_acquire(MOCK_TID_B, MOCK_LOCK_X, 0, ts));
        ts += 100;
        events.push(make_lock_contention(MOCK_TID_B, MOCK_LOCK_X, ts));
        ts += 100;

        // ── Both threads now blocked — deadlock state ──
        events.push(make_sched_switch(MOCK_TID_A, MOCK_TID_B, 0, ts));
        ts += 100;
        events.push(make_sched_switch(MOCK_TID_B, MOCK_TID_A, 0, ts));

        events
    }

    /// Generates a data-race scenario.
    ///
    /// Two threads perform conflicting operations on the same resource without
    /// any synchronization. The DPOR engine detects this as concurrent conflicting
    /// accesses (both request the same resource without a Release in between).
    ///
    /// # Event sequence
    ///
    /// ```text
    /// TID_A: ThreadSpawn → LockAcquire(X) [gets it]
    /// TID_B: ThreadSpawn → LockAcquire(X) [blocked — race!]
    /// TID_A: LockAcquire(X) again [deadlock — holds and re-requests]
    /// ```
    pub fn generate_data_race(&self) -> Vec<RawProbeEvent> {
        let mut ts = BASE_CLOCK_NS + self.seed % 1_000_000;
        let mut events = Vec::with_capacity(10);

        events.push(make_thread_spawn(MOCK_TID_A, 0, ts));
        ts += 100;
        events.push(make_thread_spawn(MOCK_TID_B, MOCK_TID_A, ts));
        ts += 100;

        // Both threads rush to acquire the same lock — data race
        events.push(make_lock_acquire(MOCK_TID_A, MOCK_LOCK_X, 0, ts));
        ts += 50;
        events.push(make_lock_acquired(MOCK_TID_A, MOCK_LOCK_X, 0, ts));
        ts += 50;

        events.push(make_lock_acquire(MOCK_TID_B, MOCK_LOCK_X, 0, ts));
        ts += 100;
        events.push(make_lock_contention(MOCK_TID_B, MOCK_LOCK_X, ts));
        ts += 100;

        // Thread A re-requests without releasing — causes self-deadlock
        // Thread A holds X, Thread B waits for X, Thread A requests X again → total block
        events.push(make_lock_acquire(MOCK_TID_A, MOCK_LOCK_X, 0, ts));
        ts += 100;
        events.push(make_lock_contention(MOCK_TID_A, MOCK_LOCK_X, ts));
        ts += 100;

        events.push(make_sched_switch(MOCK_TID_A, MOCK_TID_B, 0, ts));
        ts += 100;
        events.push(make_sched_switch(MOCK_TID_B, MOCK_TID_A, 0, ts));

        events
    }

    /// Generates a livelock / starvation scenario.
    ///
    /// Thread A repeatedly requests and releases lock X, while Thread B
    /// perpetually waits. Because Thread A keeps "winning" every Release →
    /// Re-Acquire cycle, Thread B's starvation counter climbs past
    /// `MAX_STARVATION_LIMIT` (= 10).
    ///
    /// # Event sequence
    ///
    /// ```text
    /// TID_A: ThreadSpawn → LockAcquire(X) → LockAcquired(X)
    /// TID_B: ThreadSpawn → LockAcquire(X) [blocked forever]
    /// TID_A: × 12 cycles of LockRelease(X) → LockAcquire(X) → LockAcquired(X)
    /// ```
    pub fn generate_livelock_starvation(&self) -> Vec<RawProbeEvent> {
        let mut ts = BASE_CLOCK_NS + self.seed % 1_000_000;
        let mut events = Vec::with_capacity(40);

        events.push(make_thread_spawn(MOCK_TID_A, 0, ts));
        ts += 100;
        events.push(make_thread_spawn(MOCK_TID_B, MOCK_TID_A, ts));
        ts += 100;

        // Thread A acquires the lock first
        events.push(make_lock_acquire(MOCK_TID_A, MOCK_LOCK_X, 0, ts));
        ts += 50;
        events.push(make_lock_acquired(MOCK_TID_A, MOCK_LOCK_X, 0, ts));
        ts += 50;

        // Thread B tries to acquire — immediately blocked
        events.push(make_lock_acquire(MOCK_TID_B, MOCK_LOCK_X, 0, ts));
        ts += 50;
        events.push(make_lock_contention(MOCK_TID_B, MOCK_LOCK_X, ts));
        ts += 50;

        // Thread A releases and immediately re-acquires — 12 cycles to exceed limit of 10
        // Thread B starvation counter increments each time A gets it instead of B
        for _ in 0..12 {
            events.push(make_lock_release(MOCK_TID_A, MOCK_LOCK_X, ts));
            ts += 50;
            // Thread A grabs it back before B can act (unfair scheduling)
            events.push(make_lock_acquire(MOCK_TID_A, MOCK_LOCK_X, 0, ts));
            ts += 50;
            events.push(make_lock_acquired(MOCK_TID_A, MOCK_LOCK_X, 0, ts));
            ts += 50;
            // Thread B still blocked — starvation counter increments
            events.push(make_lock_contention(MOCK_TID_B, MOCK_LOCK_X, ts));
            ts += 50;
        }

        events
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Private helpers — raw event constructors
// ─────────────────────────────────────────────────────────────────────────────

fn zeroed() -> RawProbeEvent {
    // SAFETY: RawProbeEvent is repr(C) with no non-POD fields.
    unsafe { core::mem::zeroed() }
}

fn make_thread_spawn(child_tid: u32, parent_tid: u32, ts: u64) -> RawProbeEvent {
    let mut e = zeroed();
    e.event_type = ProbeEventType::ThreadSpawn as u8;
    e.tid = child_tid;
    e.parent_tid = parent_tid as u64;
    e.timestamp_ns = ts;
    e
}

fn make_lock_acquire(tid: u32, mutex_addr: u64, latency_ns: u64, ts: u64) -> RawProbeEvent {
    let mut e = zeroed();
    e.event_type = ProbeEventType::LockAcquire as u8;
    e.tid = tid;
    e.resource_id = mutex_addr;
    e.latency_ns = latency_ns;
    e.timestamp_ns = ts;
    e
}

fn make_lock_acquired(tid: u32, mutex_addr: u64, contention_ns: u64, ts: u64) -> RawProbeEvent {
    let mut e = zeroed();
    e.event_type = ProbeEventType::LockAcquired as u8;
    e.tid = tid;
    e.resource_id = mutex_addr;
    e.latency_ns = contention_ns;
    e.timestamp_ns = ts;
    e
}

fn make_lock_release(tid: u32, mutex_addr: u64, ts: u64) -> RawProbeEvent {
    let mut e = zeroed();
    e.event_type = ProbeEventType::LockRelease as u8;
    e.tid = tid;
    e.resource_id = mutex_addr;
    e.timestamp_ns = ts;
    e
}

fn make_lock_contention(tid: u32, mutex_addr: u64, ts: u64) -> RawProbeEvent {
    let mut e = zeroed();
    e.event_type = ProbeEventType::LockContention as u8;
    e.tid = tid;
    e.resource_id = mutex_addr;
    e.timestamp_ns = ts;
    e
}

fn make_sched_switch(prev_tid: u32, next_tid: u32, cpu_id: u32, ts: u64) -> RawProbeEvent {
    let mut e = zeroed();
    e.event_type = ProbeEventType::SchedSwitch as u8;
    e.tid = prev_tid;
    e.parent_tid = next_tid as u64; // repurposed field
    e.cpu_id = cpu_id;
    e.timestamp_ns = ts;
    e
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn is_monotonic(events: &[RawProbeEvent]) -> bool {
        events
            .windows(2)
            .all(|w| w[0].timestamp_ns <= w[1].timestamp_ns)
    }

    #[test]
    fn ab_ba_timestamps_are_monotonic() {
        let src = MockProbeSource::default_seed();
        let events = src.generate_ab_ba_deadlock();
        assert!(is_monotonic(&events), "timestamps must be non-decreasing");
    }

    #[test]
    fn data_race_timestamps_are_monotonic() {
        let src = MockProbeSource::default_seed();
        let events = src.generate_data_race();
        assert!(is_monotonic(&events));
    }

    #[test]
    fn livelock_timestamps_are_monotonic() {
        let src = MockProbeSource::default_seed();
        let events = src.generate_livelock_starvation();
        assert!(is_monotonic(&events));
    }

    #[test]
    fn ab_ba_contains_both_tids_and_both_locks() {
        let src = MockProbeSource::default_seed();
        let events = src.generate_ab_ba_deadlock();

        let has_tid_a = events.iter().any(|e| e.tid == MOCK_TID_A);
        let has_tid_b = events.iter().any(|e| e.tid == MOCK_TID_B);
        let has_lock_x = events
            .iter()
            .any(|e| e.resource_id == MOCK_LOCK_X && e.tid == MOCK_TID_A);
        let has_lock_y = events
            .iter()
            .any(|e| e.resource_id == MOCK_LOCK_Y && e.tid == MOCK_TID_B);

        assert!(has_tid_a, "must include Thread A events");
        assert!(has_tid_b, "must include Thread B events");
        assert!(has_lock_x, "Thread A must touch lock X");
        assert!(has_lock_y, "Thread B must touch lock Y");
    }

    #[test]
    fn ab_ba_has_cross_lock_contention() {
        let src = MockProbeSource::default_seed();
        let events = src.generate_ab_ba_deadlock();

        // Thread A must request lock Y (which B holds)
        let a_requests_y = events.iter().any(|e| {
            e.tid == MOCK_TID_A
                && e.resource_id == MOCK_LOCK_Y
                && (e.event_type == ProbeEventType::LockAcquire as u8
                    || e.event_type == ProbeEventType::LockContention as u8)
        });
        // Thread B must request lock X (which A holds)
        let b_requests_x = events.iter().any(|e| {
            e.tid == MOCK_TID_B
                && e.resource_id == MOCK_LOCK_X
                && (e.event_type == ProbeEventType::LockAcquire as u8
                    || e.event_type == ProbeEventType::LockContention as u8)
        });

        assert!(a_requests_y, "Thread A must contend on lock Y");
        assert!(b_requests_x, "Thread B must contend on lock X");
    }

    #[test]
    fn livelock_has_enough_cycles_to_trigger_starvation() {
        let src = MockProbeSource::default_seed();
        let events = src.generate_livelock_starvation();

        // Count how many times Thread A releases lock X
        let release_count = events
            .iter()
            .filter(|e| e.tid == MOCK_TID_A && e.event_type == ProbeEventType::LockRelease as u8)
            .count();

        // Must be at least 11 to exceed MAX_STARVATION_LIMIT = 10
        assert!(
            release_count >= 11,
            "need ≥ 11 release cycles to exceed starvation limit; got {}",
            release_count
        );
    }

    #[test]
    fn different_seeds_produce_different_timestamps() {
        let e1 = MockProbeSource::new(0).generate_ab_ba_deadlock();
        let e2 = MockProbeSource::new(9999).generate_ab_ba_deadlock();
        assert_ne!(
            e1[0].timestamp_ns, e2[0].timestamp_ns,
            "different seeds must produce different base timestamps"
        );
    }

    #[test]
    fn all_event_types_are_valid_discriminants() {
        let src = MockProbeSource::default_seed();
        let all: Vec<_> = [
            src.generate_ab_ba_deadlock(),
            src.generate_data_race(),
            src.generate_livelock_starvation(),
        ]
        .into_iter()
        .flatten()
        .collect();

        for e in &all {
            assert!(
                (1..=11).contains(&e.event_type),
                "invalid event_type: {}",
                e.event_type
            );
        }
    }
}
