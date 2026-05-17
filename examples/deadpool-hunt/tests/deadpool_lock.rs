//! deadpool v0.13.0 형식 검증
//!
//! Scenario A: 기본 pool.get() — 단일 락 CLEAN 확인
//! Scenario B: 이중 락 확장 — pool_state + conn_meta AB-BA BugFound 확인

use std::future::Future;

use deadpool::managed::{Manager, Metrics, Object, RecycleResult};
use laplace_probe_sdk::{
    run_verification_from, set_probe_sender, set_probe_thread_id, ProbeEvent, ProbeSessionConfig,
    TrackedStdMutex,
};
use std::sync::{mpsc, Arc};

/// Mock connection manager for deadpool
#[derive(Clone)]
struct MockManager;

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

    // Lock A: pool_state (TrackedStdMutex)
    // Lock B: conn_meta (TrackedStdMutex)
    let pool_state = Arc::new(TrackedStdMutex::new(0u64, "pool_state"));
    let conn_meta = Arc::new(TrackedStdMutex::new(0u64, "conn_meta"));

    let mut handles = Vec::new();

    // Thread 0: pool_state → conn_meta (A → B)
    {
        let ps = pool_state.clone();
        let cm = conn_meta.clone();
        let tx2 = tx.clone();
        handles.push(std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(0u64);
            let _ga = ps.lock();
            let _gb = cm.lock();
        }));
    }

    // Thread 1: conn_meta → pool_state (B → A, 역순!)
    {
        let ps = pool_state.clone();
        let cm = conn_meta.clone();
        let tx2 = tx.clone();
        handles.push(std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(1u64);
            let _gb = cm.lock();
            let _ga = ps.lock();
        }));
    }

    drop(tx);
    for h in handles {
        h.join().expect("thread panic");
    }

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
    println!("자원: {:?}", resource_names);

    assert_eq!(resource_names.len(), 2, "자원 2개 필수");

    let config = ProbeSessionConfig {
        write_ard: true,
        output_dir: std::env::temp_dir().to_string_lossy().into_owned(),
        ..ProbeSessionConfig::default()
    };
    run_verification_from(&events, "deadpool_dual_lock_ab_ba", &config).assert_bug();
}
