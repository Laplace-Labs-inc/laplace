#![deny(clippy::all, clippy::pedantic)]
#![allow(clippy::doc_markdown)]

//! Phase 4-C: Known Deadlock Pattern — mobc + HealthChecker (multi-lock AB-BA)
//!
//! **Scenario**: Real mobc v0.9.0 with additional health check mechanism that acquires
//! locks in reverse order, demonstrating realistic deadlock on connection pools.
//!
//! **Architecture**:
//! - Lock A: pool.internals (connection state) — TrackedMutex
//! - Lock B: health_state (connection metadata) — TrackedMutex
//! - Thread 1 (Pool::get): A → B
//! - Thread 2 (HealthChecker): B → A (reverse!)
//!
//! **Expected**: CLEAN — mobc releases `pool.internals` before `get` returns,
//! so thread 0 never holds Lock A while acquiring Lock B; the tracked-lock
//! pattern has no AB-BA cycle. See the assertion comment for scope notes.

use async_trait::async_trait;
use laplace_probe_sdk::TrackedMutex;
use std::sync::Arc;

// ── Health Check State (Lock B) ────────────────────────────────────────────────

struct HealthCheckState {
    last_check: i64,
}

// ── Pool With Health Checker (AB-BA Pattern) ───────────────────────────────────

struct PoolWithHealthCheck {
    pool: mobc::Pool<MockManager>,
    health_state: TrackedMutex<HealthCheckState>, // Lock B — must be TrackedMutex for DPOR!
}

#[derive(Debug, Clone)]
struct MockManager;

#[async_trait]
impl mobc::Manager for MockManager {
    type Connection = i64;
    type Error = std::io::Error;

    async fn connect(&self) -> Result<Self::Connection, Self::Error> {
        Ok(42)
    }

    async fn check(&self, conn: Self::Connection) -> Result<Self::Connection, Self::Error> {
        Ok(conn)
    }
}

impl Default for PoolWithHealthCheck {
    fn default() -> Self {
        let manager = MockManager;
        let pool = mobc::Pool::builder().max_open(4).build(manager);
        Self {
            pool,
            health_state: TrackedMutex::new(HealthCheckState { last_check: 0 }, "health_state"),
        }
    }
}

// [Scenarios A & B commented out — use multi_lock_ab_ba_deadlock test instead]
// The axiom_target macro runs threads sequentially, not interleaved.
// The actual multi-lock deadlock scenario is in the multi_lock_ab_ba_deadlock test.

// ── Combined Test: 2-thread AB-BA deadlock scenario ────────────────────────────

/// Real mobc Pool with dual-lock health check scenario.
/// Thread 0: pool.get() → health check (A released before B — sequential)
/// Thread 1: health check → pool access (B → A, nested)
///
/// Expected DPOR result: CLEAN (no tracked-lock cycle; see assertion comment)
#[test]
fn multi_lock_ab_ba_deadlock() {
    use laplace_probe_sdk::{
        clear_probe_sender, run_verification_from, set_probe_sender, set_probe_thread_id,
        ProbeEvent, ProbeSessionConfig,
    };
    use std::sync::mpsc;

    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(4096);

    let state = Arc::new(PoolWithHealthCheck::default());
    let mut handles = Vec::new();

    // Thread 0: Pool::get first (Lock A), then health check (Lock B)
    {
        let s = state.clone();
        let tx2 = tx.clone();
        handles.push(std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(0u64);
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("rt");
            rt.block_on(async {
                let _conn = s.pool.get().await.expect("pool get");
                let _health = s.health_state.lock().await;
            });
        }));
    }

    // Thread 1: Health check first (Lock B), then pool access (Lock A) — REVERSE!
    {
        let s = state.clone();
        let tx2 = tx.clone();
        handles.push(std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(1u64);
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("rt");
            rt.block_on(async {
                let mut health = s.health_state.lock().await;
                health.last_check = 1;
                // Access pool after acquiring health lock (reverse order!)
                let _conn = s.pool.get().await.expect("pool get for health");
            });
        }));
    }

    drop(tx);
    for h in handles {
        h.join().expect("thread panicked");
    }

    // set_probe_sender는 전역 슬롯에도 클론을 남기므로 수집 전에 클리어해야
    // rx.into_iter()가 종료된다.
    clear_probe_sender();
    let events: Vec<ProbeEvent> = rx.into_iter().collect();

    println!("\n[known-bug-hunt] Collected {} events:", events.len());
    for (i, e) in events.iter().enumerate() {
        println!("  [{i}] {e:?}");
    }

    let resources: std::collections::HashSet<_> = events
        .iter()
        .filter_map(|e| match e {
            ProbeEvent::LockAcquired { resource, .. } => Some(resource.as_str()),
            _ => None,
        })
        .collect();

    println!("\n자원 수 (= 내부 Mutex 수): {}", resources.len());
    println!("자원: {resources:?}");

    let config = ProbeSessionConfig {
        write_ard: true,
        output_dir: std::env::temp_dir().to_string_lossy().into_owned(),
        ..ProbeSessionConfig::default()
    };

    // Honest expectation: CLEAN. mobc's `get` releases `pool_internals` before
    // returning the connection, so thread 0 never holds Lock A while acquiring
    // Lock B — the tracked-lock pattern has no AB-BA cycle in any interleaving.
    // (Thread 0 holding the *connection* is a pool-capacity resource, not a
    // tracked lock edge, and is outside this trace's scope.) The original
    // `assert_bug` expectation encoded the retired Ki-DPOR searcher's verdict
    // and was unreachable since 2026-06-14 because this harness hung before
    // the assertion; the nested-hold AB-BA composition lives in
    // bb8-hunt/bb8_lock_ordering.rs and still asserts BugFound.
    run_verification_from(&events, "multi_lock_ab_ba", &config).assert_clean();
}
