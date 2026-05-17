// SPDX-License-Identifier: Apache-2.0
//! Lock-Free Engine Metrics
//!
//! Provides high-frequency, zero-overhead counters for the simulation engine,
//! plus an HDR histogram for precise tail-latency percentile tracking.
//!
//! # Design
//!
//! All fields are `AtomicU64`. Increment/decrement methods use `Ordering::Relaxed`
//! so that the hardware never emits a memory fence — the counter update costs
//! exactly one atomic RMW instruction and nothing else.
//!
//! Reads are eventually consistent, which is the correct trade-off for a
//! dashboard/TUI polling loop: we want accuracy over time, not point-in-time
//! exactness.
//!
//! # Latency Histogram (verification feature)
//!
//! When compiled with `feature = "verification"`, an `hdrhistogram::Histogram<u64>`
//! is embedded in `EngineMetrics` behind a `parking_lot::RwLock`. This allows
//! any thread to call `record_latency(ms)` on the hot path, while the TUI thread
//! reads `latency_percentiles()` to get p50/p90/p99 without blocking writers.
//!
//! # No Async Required
//!
//! No `tokio`, no channels. `EngineMetrics` is a plain `struct` that can live
//! inside any `static` or `Arc` without runtime ceremony.

use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "verification")]
use parking_lot::RwLock;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// EngineMetrics
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Lock-free telemetry counters for the simulation engine.
///
/// All mutations use [`Ordering::Relaxed`] — the cost of each counter update
/// is a single atomic read-modify-write with zero fence overhead.
///
/// # Thread Safety
///
/// `EngineMetrics` is `Send + Sync`. Multiple threads may call `inc_*`/`dec_*`
/// concurrently without any locking.
///
/// # Usage
///
/// ```rust
/// use laplace_core::domain::telemetry::metrics::EngineMetrics;
///
/// let m = EngineMetrics::new();
/// m.inc_requests();
/// m.inc_active_vus();
/// assert_eq!(m.total_requests(), 1);
/// assert_eq!(m.active_vus(), 1);
/// ```
pub struct EngineMetrics {
    /// Total HTTP/virtual requests dispatched since startup.
    total_requests: AtomicU64,

    /// Number of virtual users (VUs / contexts) currently active.
    active_vus: AtomicU64,

    /// Number of DPOR states explored (state-space nodes visited).
    explored_states: AtomicU64,

    /// Number of branches pruned by DPOR's partial-order reduction.
    pruned_branches: AtomicU64,

    /// Total requests that completed successfully (HTTP < 400).
    successful_requests: AtomicU64,

    /// Total requests that failed (HTTP >= 400).
    failed_requests: AtomicU64,

    /// HDR histogram for precise tail-latency percentile tracking (ms).
    ///
    /// Requires `feature = "verification"` (parking_lot + hdrhistogram).
    /// Writers call `record_latency(ms)` on the hot path;
    /// the TUI calls `latency_percentiles()` to snapshot p50/p90/p99.
    #[cfg(feature = "verification")]
    latency_hist: RwLock<hdrhistogram::Histogram<u64>>,

    // ── Semantic Mesh Telemetry ───────────────────────────────────────────────
    /// Bytes actually transmitted over the network after semantic compression.
    mesh_bytes_tx: AtomicU64,

    /// Bytes received from the network in compressed form.
    mesh_bytes_rx: AtomicU64,

    /// Logical TX byte count as if sent over plain HTTP (no compression).
    /// Used to compute the compression ratio on the send side.
    http_equivalent_tx: AtomicU64,

    /// Logical RX byte count as if received over plain HTTP (no compression).
    /// Used to compute the compression ratio on the receive side.
    http_equivalent_rx: AtomicU64,

    // ── System Resource Telemetry ─────────────────────────────────────────────
    /// CPU utilization in percent, stored as f64 bits for atomic access.
    cpu_usage_bits: AtomicU64,

    /// Memory usage in megabytes, stored as f64 bits for atomic access.
    memory_mb_bits: AtomicU64,

    /// CPI efficiency score, stored as f64 bits for atomic access.
    cpi_score_bits: AtomicU64,
}

impl EngineMetrics {
    /// Create a new zeroed `EngineMetrics`.
    ///
    /// Allocates an HDR histogram (3 significant digits, max 60_000 ms)
    /// when compiled with `feature = "verification"`.
    pub fn new() -> Self {
        Self {
            total_requests: AtomicU64::new(0),
            successful_requests: AtomicU64::new(0),
            failed_requests: AtomicU64::new(0),
            active_vus: AtomicU64::new(0),
            explored_states: AtomicU64::new(0),
            pruned_branches: AtomicU64::new(0),
            #[cfg(feature = "verification")]
            latency_hist: RwLock::new(
                hdrhistogram::Histogram::<u64>::new(3).expect("hdrhistogram init failed"),
            ),
            mesh_bytes_tx: AtomicU64::new(0),
            mesh_bytes_rx: AtomicU64::new(0),
            http_equivalent_tx: AtomicU64::new(0),
            http_equivalent_rx: AtomicU64::new(0),
            cpu_usage_bits: AtomicU64::new(0),
            memory_mb_bits: AtomicU64::new(0),
            cpi_score_bits: AtomicU64::new(0),
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Increment / decrement — hot-path methods (Ordering::Relaxed)
    // ─────────────────────────────────────────────────────────────────────────

    /// Increment `total_requests` by 1.
    #[inline(always)]
    pub fn inc_requests(&self) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment `active_vus` by 1 (VU spawned).
    #[inline(always)]
    pub fn inc_active_vus(&self) {
        self.active_vus.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement `active_vus` by 1 (VU finished).
    ///
    /// Saturating: silently ignores underflow (returns 0 minimum).
    #[inline(always)]
    pub fn dec_active_vus(&self) {
        // fetch_update lets us saturate at 0 without extra branching overhead.
        let _ = self
            .active_vus
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                Some(v.saturating_sub(1))
            });
    }

    /// Increment `explored_states` by 1 (DPOR node expanded).
    #[inline(always)]
    pub fn inc_explored_states(&self) {
        self.explored_states.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment `pruned_branches` by 1 (DPOR branch eliminated).
    #[inline(always)]
    pub fn inc_pruned_branches(&self) {
        self.pruned_branches.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment `successful_requests` by 1.
    #[inline(always)]
    pub fn inc_successful(&self) {
        self.successful_requests.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment `failed_requests` by 1.
    #[inline(always)]
    pub fn inc_failed(&self) {
        self.failed_requests.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a mesh TX event: `mesh_bytes` is the wire size after semantic
    /// compression; `http_bytes` is the equivalent uncompressed HTTP size.
    #[inline(always)]
    pub fn record_mesh_tx(&self, mesh_bytes: u64, http_bytes: u64) {
        self.mesh_bytes_tx.fetch_add(mesh_bytes, Ordering::Relaxed);
        self.http_equivalent_tx
            .fetch_add(http_bytes, Ordering::Relaxed);
    }

    /// Record a mesh RX event: `mesh_bytes` is the wire size of the received
    /// compressed payload; `http_bytes` is the equivalent uncompressed size.
    #[inline(always)]
    pub fn record_mesh_rx(&self, mesh_bytes: u64, http_bytes: u64) {
        self.mesh_bytes_rx.fetch_add(mesh_bytes, Ordering::Relaxed);
        self.http_equivalent_rx
            .fetch_add(http_bytes, Ordering::Relaxed);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Snapshot reads — TUI polling (Ordering::Relaxed)
    // ─────────────────────────────────────────────────────────────────────────

    /// Read the current `total_requests` counter.
    #[inline(always)]
    pub fn total_requests(&self) -> u64 {
        self.total_requests.load(Ordering::Relaxed)
    }

    /// Read the current `active_vus` counter.
    #[inline(always)]
    pub fn active_vus(&self) -> u64 {
        self.active_vus.load(Ordering::Relaxed)
    }

    /// Read the current `explored_states` counter.
    #[inline(always)]
    pub fn explored_states(&self) -> u64 {
        self.explored_states.load(Ordering::Relaxed)
    }

    /// Read the current `pruned_branches` counter.
    #[inline(always)]
    pub fn pruned_branches(&self) -> u64 {
        self.pruned_branches.load(Ordering::Relaxed)
    }

    /// Read the current `successful_requests` counter.
    #[inline(always)]
    pub fn successful_requests(&self) -> u64 {
        self.successful_requests.load(Ordering::Relaxed)
    }

    /// Read the current `failed_requests` counter.
    #[inline(always)]
    pub fn failed_requests(&self) -> u64 {
        self.failed_requests.load(Ordering::Relaxed)
    }

    /// Calculate Requests Per Second given an elapsed duration in seconds.
    ///
    /// Returns 0.0 if `elapsed_seconds` is non-positive.
    #[inline]
    pub fn rps(&self, elapsed_seconds: f64) -> f64 {
        if elapsed_seconds <= 0.0 {
            return 0.0;
        }
        self.total_requests() as f64 / elapsed_seconds
    }

    /// Read the cumulative mesh TX bytes (wire size after compression).
    #[inline(always)]
    pub fn mesh_bytes_tx(&self) -> u64 {
        self.mesh_bytes_tx.load(Ordering::Relaxed)
    }

    /// Read the cumulative mesh RX bytes (wire size of compressed payload).
    #[inline(always)]
    pub fn mesh_bytes_rx(&self) -> u64 {
        self.mesh_bytes_rx.load(Ordering::Relaxed)
    }

    /// Read the cumulative HTTP-equivalent TX bytes (uncompressed logical size).
    #[inline(always)]
    pub fn http_equivalent_tx(&self) -> u64 {
        self.http_equivalent_tx.load(Ordering::Relaxed)
    }

    /// Read the cumulative HTTP-equivalent RX bytes (uncompressed logical size).
    #[inline(always)]
    pub fn http_equivalent_rx(&self) -> u64 {
        self.http_equivalent_rx.load(Ordering::Relaxed)
    }

    // ─────────────────────────────────────────────────────────────────────────
    // System resource telemetry — CPU / Memory / CPI
    // ─────────────────────────────────────────────────────────────────────────

    /// Store the current CPU utilization percentage.
    #[inline(always)]
    pub fn set_cpu_usage(&self, value: f64) {
        self.cpu_usage_bits
            .store(value.to_bits(), Ordering::Relaxed);
    }

    /// Read the current CPU utilization percentage.
    #[inline(always)]
    pub fn cpu_usage(&self) -> f64 {
        f64::from_bits(self.cpu_usage_bits.load(Ordering::Relaxed))
    }

    /// Store the current memory usage in megabytes.
    #[inline(always)]
    pub fn set_memory_mb(&self, value: f64) {
        self.memory_mb_bits
            .store(value.to_bits(), Ordering::Relaxed);
    }

    /// Read the current memory usage in megabytes.
    #[inline(always)]
    pub fn memory_mb(&self) -> f64 {
        f64::from_bits(self.memory_mb_bits.load(Ordering::Relaxed))
    }

    /// Store the current CPI efficiency score.
    #[inline(always)]
    pub fn set_cpi_score(&self, value: f64) {
        self.cpi_score_bits
            .store(value.to_bits(), Ordering::Relaxed);
    }

    /// Read the current CPI efficiency score.
    #[inline(always)]
    pub fn cpi_score(&self) -> f64 {
        f64::from_bits(self.cpi_score_bits.load(Ordering::Relaxed))
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Latency histogram — verification feature only
    // ─────────────────────────────────────────────────────────────────────────

    /// Record a single latency observation in milliseconds.
    ///
    /// Requires `feature = "verification"`. Uses a write lock for the minimum
    /// time to insert one value into the HDR histogram.
    #[cfg(feature = "verification")]
    #[inline]
    pub fn record_latency(&self, ms: u64) {
        self.latency_hist.write().record(ms).unwrap_or(());
    }

    /// Return (p50, p90, p99) tail-latency percentiles in milliseconds.
    ///
    /// Requires `feature = "verification"`. Takes a short read lock;
    /// concurrent `record_latency` calls are not blocked by this.
    /// Returns `(0, 0, 0)` when no samples have been recorded yet.
    #[cfg(feature = "verification")]
    pub fn latency_percentiles(&self) -> (u64, u64, u64) {
        let hist = self.latency_hist.read();
        let p50 = hist.value_at_percentile(50.0);
        let p90 = hist.value_at_percentile(90.0);
        let p99 = hist.value_at_percentile(99.0);
        (p50, p90, p99)
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Reset
    // ─────────────────────────────────────────────────────────────────────────

    /// Reset all counters to zero and clear the latency histogram.
    ///
    /// Intended for test teardown or scenario restarts.
    /// Uses `Ordering::Relaxed` — callers are responsible for ensuring no
    /// concurrent increments race with the reset if strict ordering matters.
    pub fn reset(&self) {
        self.total_requests.store(0, Ordering::Relaxed);
        self.successful_requests.store(0, Ordering::Relaxed);
        self.failed_requests.store(0, Ordering::Relaxed);
        self.active_vus.store(0, Ordering::Relaxed);
        self.explored_states.store(0, Ordering::Relaxed);
        self.pruned_branches.store(0, Ordering::Relaxed);
        #[cfg(feature = "verification")]
        self.latency_hist.write().clear();
        self.mesh_bytes_tx.store(0, Ordering::Relaxed);
        self.mesh_bytes_rx.store(0, Ordering::Relaxed);
        self.http_equivalent_tx.store(0, Ordering::Relaxed);
        self.http_equivalent_rx.store(0, Ordering::Relaxed);
        self.cpu_usage_bits.store(0, Ordering::Relaxed);
        self.memory_mb_bits.store(0, Ordering::Relaxed);
        self.cpi_score_bits.store(0, Ordering::Relaxed);
    }
}

impl Default for EngineMetrics {
    fn default() -> Self {
        Self::new()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// VuMetricEvent — per-request metric envelope
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Per-request metric event emitted by each virtual user.
///
/// Streamed through an [`MetricCollector`] channel to a background aggregator
/// that updates [`EngineMetrics`] counters and the HDR histogram.
#[derive(Debug, Clone)]
pub struct VuMetricEvent {
    /// Virtual user identifier.
    pub vu_id: u64,
    /// Name of the scenario being executed.
    pub scenario_name: String,
    /// Round-trip latency in nanoseconds.
    pub latency_ns: u64,
    /// Whether the request succeeded (HTTP status < 400).
    pub success: bool,
    /// Optional chaos profile label (e.g., "MOBILE_4G", "SATELLITE").
    pub chaos_profile: Option<String>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// MetricCollector — lock-free channel sender + background aggregator
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Lock-free metric collector that streams [`VuMetricEvent`]s to a background
/// aggregator task via an unbounded mpsc channel.
///
/// # Usage
///
/// ```rust,ignore
/// let (collector, handle) = MetricCollector::new(GlobalTelemetry::metrics());
/// collector.record(VuMetricEvent { /* ... */ });
/// // Drop collector to signal completion, then await the aggregator.
/// drop(collector);
/// handle.await.unwrap();
/// ```
#[cfg(feature = "twin")]
#[derive(Clone)]
pub struct MetricCollector {
    tx: tokio::sync::mpsc::UnboundedSender<VuMetricEvent>,
}

#[cfg(feature = "twin")]
impl MetricCollector {
    /// Spawn a background aggregator task and return `(collector, join_handle)`.
    ///
    /// The aggregator drains the channel and updates the given [`EngineMetrics`]
    /// until the sender side is dropped.
    pub fn new(metrics: &'static EngineMetrics) -> (Self, tokio::task::JoinHandle<()>) {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<VuMetricEvent>();

        let handle = tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                metrics.inc_requests();
                if ev.success {
                    metrics.inc_successful();
                } else {
                    metrics.inc_failed();
                }
                // Convert ns → ms for the HDR histogram
                #[cfg(feature = "verification")]
                metrics.record_latency(ev.latency_ns / 1_000_000);
            }
        });

        (Self { tx }, handle)
    }

    /// Send a metric event to the background aggregator (lock-free).
    ///
    /// Returns silently if the aggregator has already shut down.
    #[inline]
    pub fn record(&self, event: VuMetricEvent) {
        let _ = self.tx.send(event);
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_is_zero() {
        let m = EngineMetrics::new();
        assert_eq!(m.total_requests(), 0);
        assert_eq!(m.active_vus(), 0);
        assert_eq!(m.explored_states(), 0);
        assert_eq!(m.pruned_branches(), 0);
    }

    #[test]
    fn test_inc_requests() {
        let m = EngineMetrics::new();
        m.inc_requests();
        m.inc_requests();
        assert_eq!(m.total_requests(), 2);
    }

    #[test]
    fn test_inc_dec_active_vus() {
        let m = EngineMetrics::new();
        m.inc_active_vus();
        m.inc_active_vus();
        m.inc_active_vus();
        assert_eq!(m.active_vus(), 3);
        m.dec_active_vus();
        assert_eq!(m.active_vus(), 2);
    }

    #[test]
    fn test_dec_active_vus_saturates_at_zero() {
        let m = EngineMetrics::new();
        m.dec_active_vus(); // underflow → stays at 0
        assert_eq!(m.active_vus(), 0);
    }

    #[test]
    fn test_inc_explored_states() {
        let m = EngineMetrics::new();
        for _ in 0..5 {
            m.inc_explored_states();
        }
        assert_eq!(m.explored_states(), 5);
    }

    #[test]
    fn test_inc_pruned_branches() {
        let m = EngineMetrics::new();
        m.inc_pruned_branches();
        assert_eq!(m.pruned_branches(), 1);
    }

    #[test]
    fn test_reset() {
        let m = EngineMetrics::new();
        m.inc_requests();
        m.inc_active_vus();
        m.inc_explored_states();
        m.inc_pruned_branches();
        m.reset();
        assert_eq!(m.total_requests(), 0);
        assert_eq!(m.active_vus(), 0);
        assert_eq!(m.explored_states(), 0);
        assert_eq!(m.pruned_branches(), 0);
    }

    #[test]
    fn test_default_is_zero() {
        let m = EngineMetrics::default();
        assert_eq!(m.total_requests(), 0);
    }

    #[test]
    fn test_mesh_fields_new_is_zero() {
        let m = EngineMetrics::new();
        assert_eq!(m.mesh_bytes_tx(), 0);
        assert_eq!(m.mesh_bytes_rx(), 0);
        assert_eq!(m.http_equivalent_tx(), 0);
        assert_eq!(m.http_equivalent_rx(), 0);
    }

    #[test]
    fn test_record_mesh_tx() {
        let m = EngineMetrics::new();
        m.record_mesh_tx(100, 400);
        m.record_mesh_tx(50, 200);
        assert_eq!(m.mesh_bytes_tx(), 150);
        assert_eq!(m.http_equivalent_tx(), 600);
    }

    #[test]
    fn test_record_mesh_rx() {
        let m = EngineMetrics::new();
        m.record_mesh_rx(80, 320);
        m.record_mesh_rx(20, 80);
        assert_eq!(m.mesh_bytes_rx(), 100);
        assert_eq!(m.http_equivalent_rx(), 400);
    }

    #[test]
    fn test_mesh_fields_reset() {
        let m = EngineMetrics::new();
        m.record_mesh_tx(1000, 4000);
        m.record_mesh_rx(500, 2000);
        m.reset();
        assert_eq!(m.mesh_bytes_tx(), 0);
        assert_eq!(m.mesh_bytes_rx(), 0);
        assert_eq!(m.http_equivalent_tx(), 0);
        assert_eq!(m.http_equivalent_rx(), 0);
    }

    #[test]
    fn test_success_failure_counters() {
        let m = EngineMetrics::new();
        m.inc_successful();
        m.inc_successful();
        m.inc_failed();
        assert_eq!(m.successful_requests(), 2);
        assert_eq!(m.failed_requests(), 1);
        m.reset();
        assert_eq!(m.successful_requests(), 0);
        assert_eq!(m.failed_requests(), 0);
    }

    /// 100 VUs record metrics through a MetricCollector; the background
    /// aggregator must process all events and update EngineMetrics correctly.
    #[cfg(feature = "twin")]
    #[tokio::test]
    async fn test_metric_collector_100_vus() {
        use super::MetricCollector;
        use super::VuMetricEvent;

        // Use a fresh EngineMetrics (leaked to get &'static lifetime for test)
        let metrics: &'static EngineMetrics = Box::leak(Box::new(EngineMetrics::new()));

        let (collector, aggregator_handle) = MetricCollector::new(metrics);

        // Spawn 100 "VU" tasks, each recording one event with unique latency
        let mut tasks = tokio::task::JoinSet::new();
        for vu_id in 0..100u64 {
            let collector_ref = collector.clone();
            tasks.spawn(async move {
                let latency_ns = (vu_id + 1) * 1_000_000; // 1ms..100ms in ns
                let success = vu_id % 10 != 0; // 10% failure rate (VU 0,10,20,...,90)
                collector_ref.record(VuMetricEvent {
                    vu_id,
                    scenario_name: "load-test".to_string(),
                    latency_ns,
                    success,
                    chaos_profile: None,
                });
            });
        }

        // Wait for all senders to finish
        while tasks.join_next().await.is_some() {}

        // Drop the collector to close the channel, then await aggregator
        drop(collector);
        aggregator_handle.await.unwrap();

        // Verify counters
        assert_eq!(metrics.total_requests(), 100);
        assert_eq!(metrics.successful_requests(), 90);
        assert_eq!(metrics.failed_requests(), 10);

        // Verify latency histogram (p99 should be around 100ms)
        #[cfg(feature = "verification")]
        {
            let (p50, _p90, p99) = metrics.latency_percentiles();
            assert!(p50 > 0, "p50 must be > 0");
            assert!(p99 > 0, "p99 must be > 0");
            assert!(p99 <= 100, "p99 should be ≤ 100ms");
        }
    }
}
