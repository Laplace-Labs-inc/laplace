//! deadpool v0.13.0 형식 검증
//!
//! Scenario A: 기본 pool.get() — 단일 락 CLEAN 확인
//! Scenario B: 이중 락 확장 — pool_state + conn_meta AB-BA BugFound 확인

use std::future::Future;

use deadpool::managed::{Manager, Metrics, Object, RecycleResult};
use deadpool_hunt::program::{deadpool_ab_ba_program, ModelLock, AB_BA_RESOURCES};
use laplace_probe_sdk::{
    clear_probe_sender, run_verification_from, set_probe_sender, set_probe_thread_id, ProbeEvent,
    ProbeSessionConfig, TrackedStdMutex,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};

/// Toy-path lock surface: wraps `TrackedStdMutex` so the shared program body
/// emits `ProbeEvent`s for the passive scanner.
struct ToyLock(Arc<TrackedStdMutex<()>>);

impl ModelLock for ToyLock {
    fn hold(&self, inner: &mut dyn FnMut()) {
        let _guard = self.0.lock();
        inner();
    }
}

/// Mock connection manager for deadpool
#[derive(Clone)]
struct MockManager;

// deadpool's `Manager` trait declares the returned futures with an explicit
// `+ Send` bound, which the `async fn` desugaring cannot express in a trait impl.
#[allow(clippy::manual_async_fn)]
impl Manager for MockManager {
    type Type = u64;
    type Error = std::io::Error;

    fn create(&self) -> impl Future<Output = Result<Self::Type, Self::Error>> + Send {
        async { Ok(42u64) }
    }

    fn recycle(
        &self,
        _conn: &mut Self::Type,
        _metrics: &Metrics,
    ) -> impl Future<Output = RecycleResult<Self::Error>> + Send {
        async { Ok(()) }
    }
}

// ── Scenario A: 단일 락 CLEAN baseline ──────────────────────────────────────────
/// deadpool 기본 pool.get() 동시 접근 — CLEAN 기대
#[test]
fn deadpool_pool_get_clean() {
    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(4096);

    let pool = Arc::new(
        deadpool::managed::Pool::builder(MockManager)
            .max_size(4)
            .build()
            .expect("pool build"),
    );

    let mut handles = Vec::new();

    for i in 0..2 {
        let p = pool.clone();
        let tx2 = tx.clone();
        handles.push(std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(i as u64);
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("rt");
            rt.block_on(async {
                let _conn: Object<MockManager> = p.get().await.expect("deadpool get");
                // conn drop → 자동 반환
            });
        }));
    }

    drop(tx);
    for h in handles {
        h.join().expect("thread panic");
    }

    // set_probe_sender는 전역 슬롯에도 클론을 남기므로 수집 전에 클리어해야
    // rx.into_iter()가 종료된다.
    clear_probe_sender();
    let events: Vec<ProbeEvent> = rx.into_iter().collect();
    println!("\n[deadpool-hunt A] Collected {} events:", events.len());
    for (i, e) in events.iter().enumerate() {
        println!("  [{i}] {e:?}");
    }

    let resource_count = events
        .iter()
        .filter_map(|e| match e {
            ProbeEvent::LockAcquired { resource, .. } => Some(resource.as_str()),
            _ => None,
        })
        .collect::<std::collections::HashSet<_>>()
        .len();
    println!("자원 수: {resource_count}");

    let config = ProbeSessionConfig::default();
    run_verification_from(&events, "deadpool_pool_get_clean", &config).assert_clean();
}

// ── Scenario B: 이중 락 AB-BA BugFound ─────────────────────────────────────────
/// deadpool + 외부 meta lock — AB-BA 이중 락 → BugFound 기대
#[test]
fn deadpool_dual_lock_ab_ba() {
    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(4096);

    // Drive the SAME single-source program body the private engine routes
    // (`deadpool_hunt::program::deadpool_ab_ba_program`). The toy path supplies a
    // `TrackedStdMutex`-backed lock surface (emits ProbeEvents) and a sequential
    // `std::thread::spawn` facade. The public fallback is a passive lock-order
    // scanner, so running the two orderings sequentially detects the AB-BA cycle
    // without letting the engine-unlinked test deadlock the test process.
    let make_lock = |name: &'static str| -> Arc<dyn ModelLock> {
        Arc::new(ToyLock(Arc::new(TrackedStdMutex::new((), name))))
    };
    let next_thread = AtomicU64::new(0);
    let spawn = |body: Box<dyn FnOnce() + Send + 'static>| {
        let tx2 = tx.clone();
        let thread_id = next_thread.fetch_add(1, Ordering::SeqCst);
        std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(thread_id);
            body();
        })
        .join()
        .expect("thread panic");
    };

    deadpool_ab_ba_program(&make_lock, &spawn);

    drop(tx);

    // set_probe_sender는 전역 슬롯에도 클론을 남기므로 수집 전에 클리어해야
    // rx.into_iter()가 종료된다.
    clear_probe_sender();
    let events: Vec<ProbeEvent> = rx.into_iter().collect();
    println!("\n[deadpool-hunt B] Collected {} events:", events.len());
    for (i, e) in events.iter().enumerate() {
        println!("  [{i}] {e:?}");
    }

    let resource_names: std::collections::HashSet<_> = events
        .iter()
        .filter_map(|e| match e {
            ProbeEvent::LockAcquired { resource, .. } => Some(resource.as_str()),
            _ => None,
        })
        .collect();
    println!("자원 수: {}", resource_names.len());
    println!("자원: {resource_names:?}");

    assert_eq!(
        resource_names.len(),
        AB_BA_RESOURCES.len(),
        "shared program declares {} resources",
        AB_BA_RESOURCES.len()
    );

    let config = ProbeSessionConfig {
        write_ard: true,
        output_dir: std::env::temp_dir().to_string_lossy().into_owned(),
        ..ProbeSessionConfig::default()
    };
    run_verification_from(&events, "deadpool_dual_lock_ab_ba", &config).assert_bug();
}
