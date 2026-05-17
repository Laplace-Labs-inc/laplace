//! Demo Server — intentional concurrency bugs for Laplace Axiom / Kraken demos.
//!
//! Endpoints:
//!
//! | Route          | Bug pattern             | Purpose                                  |
//! |----------------|-------------------------|------------------------------------------|
//! | `POST /book_ticket` | TOCTOU data race   | Original race-condition demo             |
//! | `POST /deadlock`    | AB-BA deadlock     | Stable deadlock demo (no process crash)  |
//!
//! The `/deadlock` endpoint is the recommended demo target for live DPOR
//! verification: it freezes two concurrent tasks indefinitely instead of
//! crashing the process with an integer underflow, giving the `laplace-mesh`
//! SDK enough time to flush all QUIC packets to the Axiom engine.

use axum::{extract::State, response::Json, routing::post, Router};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};
use tracing::info;

use laplace_probe::{laplace_probe_init, EventContext, ProbeEvent, FFI_ABI_VERSION};

fn yield_to_axiom(_ctx: EventContext) {}

// ── Constants ─────────────────────────────────────────────────────────────────

const INITIAL_TICKETS: usize = 100;

/// Monotonically increasing request counter used as a proxy thread ID.
///
/// Each concurrent request to `/deadlock` gets its own unique ID so the
/// Axiom DPOR engine can distinguish which "thread" acquired which lock.
static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

// ── AppState ──────────────────────────────────────────────────────────────────

/// Shared application state injected into every handler via `axum::extract::State`.
#[derive(Clone)]
struct AppState {
    // ── /book_ticket state ────────────────────────────────────────────────────
    /// Ticket count protected by an RwLock to allow TOCTOU races.
    tickets: Arc<tokio::sync::RwLock<usize>>,

    // ── /deadlock state ───────────────────────────────────────────────────────
    /// Lock A for the AB-BA deadlock scenario.
    lock_a: Arc<Mutex<()>>,
    /// Lock B for the AB-BA deadlock scenario.
    lock_b: Arc<Mutex<()>>,
}

// ── /book_ticket ──────────────────────────────────────────────────────────────

/// Intentional TOCTOU race condition — Read-Modify-Write without re-validation.
///
/// The `sleep` between read and write creates a window where multiple concurrent
/// tasks observe the same `current` count and all decrement without one seeing
/// the other's write, eventually underflowing to `usize::MAX`.
async fn book_ticket(State(state): State<AppState>) -> Json<Value> {
    yield_to_axiom(EventContext::new(ProbeEvent::Custom {
        name: "ticket_read_before".to_string(),
        metadata: json!({ "phase": "read" }),
    }));

    // Step A: read (shared) — multiple tasks pass through simultaneously
    let current = *state.tickets.read().await;

    if current == 0 {
        return Json(json!({ "status": "sold_out", "remaining": 0 }));
    }

    // Step B: artificial delay — other tasks read the same stale value here
    sleep(Duration::from_millis(10)).await;

    yield_to_axiom(EventContext::new(ProbeEvent::Custom {
        name: "ticket_write_before".to_string(),
        metadata: json!({ "phase": "write", "read_value": current }),
    }));

    // Step C: write without re-checking — the race condition
    let mut tickets = state.tickets.write().await;
    *tickets -= 1; // panics on underflow when current was already 0
    let remaining = *tickets;
    drop(tickets);

    info!(remaining, "ticket booked (race window was open)");

    Json(json!({ "status": "booked", "remaining": remaining }))
}

// ── /deadlock ─────────────────────────────────────────────────────────────────

/// AB-BA deadlock endpoint.
///
/// Each incoming request is assigned a unique request ID that acts as its
/// "thread ID" for the Axiom DPOR engine. The acquisition order of Lock A and
/// Lock B is randomised using the low bits of the current nanosecond timestamp:
///
/// ```text
/// Request R1 (odd ns):   acquires Lock A → sleeps 50ms → waits for Lock B  ← blocked
/// Request R2 (even ns):  acquires Lock B → sleeps 50ms → waits for Lock A  ← blocked
/// ```
///
/// After `sleep(50ms)` both requests hold one lock and are waiting for the
/// other's lock — a classic circular wait.  Neither Tokio task spins; they
/// both suspend via `.await`, keeping the process alive while the `laplace-mesh`
/// SDK background task flushes the `LockAcquired` events over QUIC.
///
/// # Probe events emitted
///
/// | Event            | When                              |
/// |------------------|-----------------------------------|
/// | `LockAcquired`   | Immediately after `.lock().await` |
/// | `LockReleased`   | Immediately before `drop(guard)`  |
async fn deadlock_handler(State(state): State<AppState>) -> Json<Value> {
    let tid = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);

    // Coin flip: use the nanosecond timestamp parity to choose lock order.
    // Concurrent requests arriving within the same millisecond will split
    // roughly 50/50 between the two orderings, guaranteeing the AB-BA pattern
    // under any realistic load of ≥2 concurrent requests.
    let ab_order = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() % 2 == 0)
        .unwrap_or(true);

    if ab_order {
        // ── Order: Lock A → Lock B ────────────────────────────────────────
        info!(tid, "deadlock: acquiring Lock A (A→B order)");
        let guard_a = state.lock_a.lock().await;
        yield_to_axiom(EventContext::new(ProbeEvent::LockAcquired {
            thread_id: tid,
            resource: "lock_a".to_string(),
        }));

        // Sleep to guarantee the concurrent request has time to acquire Lock B
        // before we attempt to acquire it, closing the circular-wait window.
        sleep(Duration::from_millis(50)).await;

        info!(
            tid,
            "deadlock: acquiring Lock B (A→B order) — may block here"
        );
        let guard_b = state.lock_b.lock().await;
        yield_to_axiom(EventContext::new(ProbeEvent::LockAcquired {
            thread_id: tid,
            resource: "lock_b".to_string(),
        }));

        info!(tid, "deadlock: holds both locks (A+B), releasing");

        // Release in reverse order (B first, then A)
        yield_to_axiom(EventContext::new(ProbeEvent::LockReleased {
            thread_id: tid,
            resource: "lock_b".to_string(),
        }));
        drop(guard_b);

        yield_to_axiom(EventContext::new(ProbeEvent::LockReleased {
            thread_id: tid,
            resource: "lock_a".to_string(),
        }));
        drop(guard_a);

        Json(json!({ "status": "ok", "tid": tid, "order": "A→B" }))
    } else {
        // ── Order: Lock B → Lock A ────────────────────────────────────────
        info!(tid, "deadlock: acquiring Lock B (B→A order)");
        let guard_b = state.lock_b.lock().await;
        yield_to_axiom(EventContext::new(ProbeEvent::LockAcquired {
            thread_id: tid,
            resource: "lock_b".to_string(),
        }));

        sleep(Duration::from_millis(50)).await;

        info!(
            tid,
            "deadlock: acquiring Lock A (B→A order) — may block here"
        );
        let guard_a = state.lock_a.lock().await;
        yield_to_axiom(EventContext::new(ProbeEvent::LockAcquired {
            thread_id: tid,
            resource: "lock_a".to_string(),
        }));

        info!(tid, "deadlock: holds both locks (B+A), releasing");

        yield_to_axiom(EventContext::new(ProbeEvent::LockReleased {
            thread_id: tid,
            resource: "lock_a".to_string(),
        }));
        drop(guard_a);

        yield_to_axiom(EventContext::new(ProbeEvent::LockReleased {
            thread_id: tid,
            resource: "lock_b".to_string(),
        }));
        drop(guard_b);

        Json(json!({ "status": "ok", "tid": tid, "order": "B→A" }))
    }
}

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let _ = laplace_probe_init(FFI_ABI_VERSION);

    let state = AppState {
        tickets: Arc::new(tokio::sync::RwLock::new(INITIAL_TICKETS)),
        lock_a: Arc::new(Mutex::new(())),
        lock_b: Arc::new(Mutex::new(())),
    };

    let app = Router::new()
        .route("/book_ticket", post(book_ticket))
        .route("/deadlock", post(deadlock_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .expect("failed to bind port 3000");

    info!("demo-server listening on http://0.0.0.0:3000");
    info!("Endpoints:");
    info!("  POST /book_ticket  — TOCTOU race condition (may underflow and crash)");
    info!("  POST /deadlock     — AB-BA deadlock (freezes tasks, never crashes)");
    info!("Initial tickets: {INITIAL_TICKETS}");

    axum::serve(listener, app).await.expect("server error");
}
