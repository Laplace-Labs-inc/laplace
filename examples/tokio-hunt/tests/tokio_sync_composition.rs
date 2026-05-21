#![deny(clippy::all, clippy::pedantic)]

//! tokio::sync primitive composition deadlock hunt
//!
//! Ki-DPOR를 사용하여 tokio::sync의 Mutex, RwLock 조합에서
//! cross-acquisition deadlock을 탐지한다.
//!
//! Loom이 테스트하지 않는 영역: primitive 간 조합.

use laplace_probe_sdk::{
    clear_probe_sender, current_thread_id, emit, run_verification_from, set_probe_sender,
    set_probe_thread_id, ProbeEvent, ProbeSessionConfig, TrackedStdMutex, TrackedStdRwLock,
};
use std::sync::{mpsc, Arc};

// ============================================================
// Tracked Wrappers (테스트 전용)
// ============================================================

/// tokio::sync::Mutex wrapper — blocking_lock() 시 Ki-DPOR 이벤트 방출
struct TrackedTokioMutex<T> {
    inner: tokio::sync::Mutex<T>,
    name: &'static str,
}

impl<T> TrackedTokioMutex<T> {
    fn new(value: T, name: &'static str) -> Self {
        Self {
            inner: tokio::sync::Mutex::new(value),
            name,
        }
    }

    fn blocking_lock(&self) -> TrackedTokioMutexGuard<'_, T> {
        let thread_id = current_thread_id();
        let guard = self.inner.blocking_lock();
        emit(ProbeEvent::LockAcquired {
            thread_id,
            resource: self.name.to_string(),
        });
        TrackedTokioMutexGuard {
            guard,
            name: self.name,
            thread_id,
        }
    }
}

struct TrackedTokioMutexGuard<'a, T> {
    guard: tokio::sync::MutexGuard<'a, T>,
    name: &'static str,
    thread_id: u64,
}

impl<T> std::ops::Deref for TrackedTokioMutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T> std::ops::DerefMut for TrackedTokioMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

impl<T> Drop for TrackedTokioMutexGuard<'_, T> {
    fn drop(&mut self) {
        emit(ProbeEvent::LockReleased {
            thread_id: self.thread_id,
            resource: self.name.to_string(),
        });
    }
}

/// tokio::sync::RwLock wrapper — blocking_read/write 시 Ki-DPOR 이벤트 방출
struct TrackedTokioRwLock<T> {
    inner: tokio::sync::RwLock<T>,
    name: &'static str,
}

impl<T> TrackedTokioRwLock<T> {
    fn new(value: T, name: &'static str) -> Self {
        Self {
            inner: tokio::sync::RwLock::new(value),
            name,
        }
    }

    fn blocking_read(&self) -> TrackedTokioRwLockReadGuard<'_, T> {
        let thread_id = current_thread_id();
        let guard = self.inner.blocking_read();
        emit(ProbeEvent::RwLockReadAcquired {
            thread_id,
            resource: self.name.to_string(),
        });
        TrackedTokioRwLockReadGuard {
            guard,
            name: self.name,
            thread_id,
        }
    }

    fn blocking_write(&self) -> TrackedTokioRwLockWriteGuard<'_, T> {
        let thread_id = current_thread_id();
        let guard = self.inner.blocking_write();
        emit(ProbeEvent::RwLockWriteAcquired {
            thread_id,
            resource: self.name.to_string(),
        });
        TrackedTokioRwLockWriteGuard {
            guard,
            name: self.name,
            thread_id,
        }
    }
}

struct TrackedTokioRwLockReadGuard<'a, T> {
    guard: tokio::sync::RwLockReadGuard<'a, T>,
    name: &'static str,
    thread_id: u64,
}

impl<T> std::ops::Deref for TrackedTokioRwLockReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T> Drop for TrackedTokioRwLockReadGuard<'_, T> {
    fn drop(&mut self) {
        emit(ProbeEvent::RwLockReadReleased {
            thread_id: self.thread_id,
            resource: self.name.to_string(),
        });
    }
}

struct TrackedTokioRwLockWriteGuard<'a, T> {
    guard: tokio::sync::RwLockWriteGuard<'a, T>,
    name: &'static str,
    thread_id: u64,
}

impl<T> std::ops::Deref for TrackedTokioRwLockWriteGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T> std::ops::DerefMut for TrackedTokioRwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

impl<T> Drop for TrackedTokioRwLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        emit(ProbeEvent::RwLockWriteReleased {
            thread_id: self.thread_id,
            resource: self.name.to_string(),
        });
    }
}

// ============================================================
// Helper
// ============================================================

fn bug_config() -> ProbeSessionConfig {
    ProbeSessionConfig {
        max_depth: 100_000,
        write_ard: true,
        output_dir: ".".to_string(),
    }
}

fn clean_config() -> ProbeSessionConfig {
    ProbeSessionConfig {
        max_depth: 100_000,
        write_ard: false,
        output_dir: ".".to_string(),
    }
}

// ============================================================
// Scenario 1: Baseline — 단일 Mutex, 순차 접근
// 기대 결과: CLEAN (deadlock 없음)
// ============================================================

#[test]
fn tokio_mutex_single_baseline() {
    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(4096);
    let mutex = Arc::new(TrackedTokioMutex::new(0i32, "mutex_single"));

    let mut handles = Vec::new();
    for tid in 0..2u64 {
        let tx2 = tx.clone();
        let m = mutex.clone();
        handles.push(std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(tid);

            let mut guard = m.blocking_lock();
            *guard += 1;
            drop(guard);
        }));
    }

    drop(tx);
    for h in handles {
        h.join().expect("thread panicked");
    }
    clear_probe_sender();

    let events: Vec<ProbeEvent> = rx.into_iter().collect();
    println!("[S1] Collected {} events", events.len());

    let config = clean_config();
    run_verification_from(&events, "tokio_mutex_single_baseline", &config).assert_clean();

    println!("[S1] PASS — single mutex, no deadlock");
}

// ============================================================
// Scenario 2: Dual tokio::sync::Mutex AB-BA
//
// Thread 0: lock(A) → lock(B)
// Thread 1: lock(B) → lock(A)
//
// 기대 결과: BUG (Deadlock cycle)
// ============================================================

#[test]
fn tokio_dual_mutex_ab_ba() {
    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(8192);
    let mutex_a = Arc::new(TrackedTokioMutex::new(0i32, "tokio_mutex_a"));
    let mutex_b = Arc::new(TrackedTokioMutex::new(0i32, "tokio_mutex_b"));

    // Thread 0: A → B
    let tx0 = tx.clone();
    let ma0 = mutex_a.clone();
    let mb0 = mutex_b.clone();
    let h0 = std::thread::spawn(move || {
        set_probe_sender(tx0);
        set_probe_thread_id(0);

        let guard_a = ma0.blocking_lock();
        let guard_b = mb0.blocking_lock();
        let _ = *guard_a + *guard_b;
        drop(guard_b);
        drop(guard_a);
    });

    // Thread 1: B → A
    let tx1 = tx.clone();
    let ma1 = mutex_a.clone();
    let mb1 = mutex_b.clone();
    let h1 = std::thread::spawn(move || {
        set_probe_sender(tx1);
        set_probe_thread_id(1);

        let guard_b = mb1.blocking_lock();
        let guard_a = ma1.blocking_lock();
        let _ = *guard_a + *guard_b;
        drop(guard_a);
        drop(guard_b);
    });

    drop(tx);
    h0.join().expect("thread 0 panicked");
    h1.join().expect("thread 1 panicked");
    clear_probe_sender();

    let events: Vec<ProbeEvent> = rx.into_iter().collect();
    println!("[S2] Collected {} events", events.len());
    for (i, e) in events.iter().enumerate() {
        println!("  Event {}: {:?}", i, e);
    }

    let config = bug_config();
    laplace_probe_sdk::session::run_verification_from(&events, "tokio_dual_mutex_ab_ba", &config)
        .assert_bug();

    println!("[S2] PASS — dual tokio::sync::Mutex AB-BA deadlock detected");
}

// ============================================================
// Scenario 3: tokio::sync::Mutex + tokio::sync::RwLock Cross
//
// Thread 0: mutex.lock() → rwlock.write()
// Thread 1: rwlock.read() [held] → mutex.lock()
//
// DashMap 패턴의 일반화: read(A) held + write(B) vs lock(B) held + write(A)
// 기대 결과: BUG (Deadlock — Mutex와 RwLock 교차)
// ============================================================

#[test]
fn tokio_mutex_rwlock_cross() {
    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(8192);
    let mutex = Arc::new(TrackedTokioMutex::new(0i32, "tokio_mutex_x"));
    let rwlock = Arc::new(TrackedTokioRwLock::new(0i32, "tokio_rwlock_x"));

    // Thread 0: mutex → rwlock.write
    let tx0 = tx.clone();
    let m0 = mutex.clone();
    let rw0 = rwlock.clone();
    let h0 = std::thread::spawn(move || {
        set_probe_sender(tx0);
        set_probe_thread_id(0);

        let guard_m = m0.blocking_lock();
        let guard_rw = rw0.blocking_write();
        let _ = *guard_m + *guard_rw;
        drop(guard_rw);
        drop(guard_m);
    });

    // Thread 1: rwlock.read → mutex
    let tx1 = tx.clone();
    let m1 = mutex.clone();
    let rw1 = rwlock.clone();
    let h1 = std::thread::spawn(move || {
        set_probe_sender(tx1);
        set_probe_thread_id(1);

        let guard_rw = rw1.blocking_read();
        let guard_m = m1.blocking_lock();
        let _ = *guard_m + *guard_rw;
        drop(guard_m);
        drop(guard_rw);
    });

    drop(tx);
    h0.join().expect("thread 0 panicked");
    h1.join().expect("thread 1 panicked");
    clear_probe_sender();

    let events: Vec<ProbeEvent> = rx.into_iter().collect();
    println!("[S3] Collected {} events", events.len());
    for (i, e) in events.iter().enumerate() {
        println!("  Event {}: {:?}", i, e);
    }

    let config = bug_config();
    laplace_probe_sdk::session::run_verification_from(&events, "tokio_mutex_rwlock_cross", &config)
        .assert_bug();

    println!("[S3] PASS — Mutex+RwLock cross deadlock detected");
}

// ============================================================
// Scenario 4: Dual tokio::sync::RwLock Read-Write Cross
//
// DashMap cross-shard 패턴의 tokio::sync 재현:
// Thread 0: rwlock_a.read() [held] → rwlock_b.write()
// Thread 1: rwlock_b.read() [held] → rwlock_a.write()
//
// 기대 결과: BUG (DashMap과 동일한 구조)
// ============================================================

#[test]
fn tokio_dual_rwlock_read_write_cross() {
    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(8192);
    let rwlock_a = Arc::new(TrackedTokioRwLock::new(0i32, "tokio_rwlock_a"));
    let rwlock_b = Arc::new(TrackedTokioRwLock::new(0i32, "tokio_rwlock_b"));

    // Thread 0: read(A) → write(B)
    let tx0 = tx.clone();
    let rwa0 = rwlock_a.clone();
    let rwb0 = rwlock_b.clone();
    let h0 = std::thread::spawn(move || {
        set_probe_sender(tx0);
        set_probe_thread_id(0);

        let read_a = rwa0.blocking_read();
        let mut write_b = rwb0.blocking_write();
        *write_b = *read_a + 1;
        drop(write_b);
        drop(read_a);
    });

    // Thread 1: read(B) → write(A)
    let tx1 = tx.clone();
    let rwa1 = rwlock_a.clone();
    let rwb1 = rwlock_b.clone();
    let h1 = std::thread::spawn(move || {
        set_probe_sender(tx1);
        set_probe_thread_id(1);

        let read_b = rwb1.blocking_read();
        let mut write_a = rwa1.blocking_write();
        *write_a = *read_b + 1;
        drop(write_a);
        drop(read_b);
    });

    drop(tx);
    h0.join().expect("thread 0 panicked");
    h1.join().expect("thread 1 panicked");
    clear_probe_sender();

    let events: Vec<ProbeEvent> = rx.into_iter().collect();
    println!("[S4] Collected {} events", events.len());
    for (i, e) in events.iter().enumerate() {
        println!("  Event {}: {:?}", i, e);
    }

    let config = bug_config();
    laplace_probe_sdk::session::run_verification_from(
        &events,
        "tokio_dual_rwlock_read_write_cross",
        &config,
    )
    .assert_bug();

    println!("[S4] PASS — dual RwLock read-write cross deadlock (DashMap analog)");
}

// ============================================================
// Scenario 5: 3-Lock Chain Deadlock
//
// Thread 0: lock(A) → lock(B)
// Thread 1: lock(B) → lock(C)
// Thread 2: lock(C) → lock(A)
//
// 3-way circular dependency. DashMap보다 복잡한 사이클.
// 기대 결과: BUG (3-thread deadlock cycle)
// ============================================================

#[test]
fn tokio_three_lock_chain() {
    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(16384);
    let mutex_a = Arc::new(TrackedTokioMutex::new(0i32, "tokio_chain_a"));
    let mutex_b = Arc::new(TrackedTokioMutex::new(0i32, "tokio_chain_b"));
    let mutex_c = Arc::new(TrackedTokioMutex::new(0i32, "tokio_chain_c"));

    let mut handles = Vec::new();

    // Thread 0: A → B
    {
        let tx2 = tx.clone();
        let a = mutex_a.clone();
        let b = mutex_b.clone();
        handles.push(std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(0);
            let _ga = a.blocking_lock();
            let _gb = b.blocking_lock();
        }));
    }

    // Thread 1: B → C
    {
        let tx2 = tx.clone();
        let b = mutex_b.clone();
        let c = mutex_c.clone();
        handles.push(std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(1);
            let _gb = b.blocking_lock();
            let _gc = c.blocking_lock();
        }));
    }

    // Thread 2: C → A
    {
        let tx2 = tx.clone();
        let c = mutex_c.clone();
        let a = mutex_a.clone();
        handles.push(std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(2);
            let _gc = c.blocking_lock();
            let _ga = a.blocking_lock();
        }));
    }

    drop(tx);
    for h in handles {
        h.join().expect("thread panicked");
    }
    clear_probe_sender();

    let events: Vec<ProbeEvent> = rx.into_iter().collect();
    println!("[S5] Collected {} events", events.len());
    for (i, e) in events.iter().enumerate() {
        println!("  Event {}: {:?}", i, e);
    }

    let config = bug_config();
    laplace_probe_sdk::session::run_verification_from(&events, "tokio_three_lock_chain", &config)
        .assert_bug();

    println!("[S5] PASS — 3-lock chain deadlock detected");
}

// ============================================================
// Scenario 6: Real-World Pattern — Cache + Fetch Lock
//
// 실제 프로덕션 패턴: RwLock(캐시) + Mutex(fetch 중복 방지)
//
// Thread 0: rwlock.read() [캐시 조회, held] → mutex.lock() [fetch 시작]
// Thread 1: mutex.lock() [fetch 중, held] → rwlock.write() [캐시 갱신]
//
// 실제 서비스에서 자연 발생하는 패턴.
// 기대 결과: BUG
// ============================================================

#[test]
fn tokio_cache_fetch_lock_deadlock() {
    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(8192);
    let cache = Arc::new(TrackedTokioRwLock::new(
        std::collections::HashMap::<String, String>::new(),
        "cache_rwlock",
    ));
    let fetch_lock = Arc::new(TrackedTokioMutex::new((), "fetch_mutex"));

    // Thread 0: "읽기 중 fetch 요청" 패턴
    // cache.read() held → fetch_lock.lock()
    let tx0 = tx.clone();
    let c0 = cache.clone();
    let f0 = fetch_lock.clone();
    let h0 = std::thread::spawn(move || {
        set_probe_sender(tx0);
        set_probe_thread_id(0);

        let read_guard = c0.blocking_read();
        let _val = read_guard.get("key");
        // 캐시 미스 — fetch lock 획득 시도 (read guard 아직 보유)
        let _fetch = f0.blocking_lock();
        drop(_fetch);
        drop(read_guard);
    });

    // Thread 1: "fetch 완료 후 캐시 갱신" 패턴
    // fetch_lock.lock() held → cache.write()
    let tx1 = tx.clone();
    let c1 = cache.clone();
    let f1 = fetch_lock.clone();
    let h1 = std::thread::spawn(move || {
        set_probe_sender(tx1);
        set_probe_thread_id(1);

        let _fetch = f1.blocking_lock();
        // fetch 완료, 캐시에 쓰기 (fetch lock 보유 중)
        let mut write_guard = c1.blocking_write();
        write_guard.insert("key".to_string(), "value".to_string());
        drop(write_guard);
        drop(_fetch);
    });

    drop(tx);
    h0.join().expect("thread 0 panicked");
    h1.join().expect("thread 1 panicked");
    clear_probe_sender();

    let events: Vec<ProbeEvent> = rx.into_iter().collect();
    println!("[S6] Collected {} events", events.len());
    for (i, e) in events.iter().enumerate() {
        println!("  Event {}: {:?}", i, e);
    }

    let config = bug_config();
    laplace_probe_sdk::session::run_verification_from(
        &events,
        "tokio_cache_fetch_lock_deadlock",
        &config,
    )
    .assert_bug();

    println!("[S6] PASS — cache+fetch lock deadlock (real-world pattern)");
}

// ============================================================
// Scenario 7: tokio::sync::watch + Mutex — Check-Then-Act Race
//
// watch 채널로 상태를 구독하면서 Mutex로 보호된 데이터를 갱신하는 패턴.
// 알려진 버그: GitHub #3168 — watch receiver가 stale 값 읽기 후 행동.
//
// Thread 0 (Reader): watch.borrow() [읽기, held] → mutex.lock() [업데이트]
// Thread 1 (Writer): mutex.lock() [쓰기, held] → watch_tx.send() [알림]
//
// 기대 결과: BUG (Reader가 stale 값 확인 후 mutex 획득 → Writer가 이미 갱신)
// ============================================================

#[test]
fn tokio_watch_mutex_race() {
    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(8192);
    let watch_state = Arc::new(TrackedStdRwLock::new(0_u64, "watch_state"));
    let shared_state = Arc::new(TrackedStdMutex::new(0_u64, "shared_state"));

    {
        let tx0 = tx.clone();
        let watch = watch_state.clone();
        let shared = shared_state.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx0);
            set_probe_thread_id(0);

            let watch_guard = watch.read();
            let stale_value = *watch_guard;
            let mut shared_guard = shared.lock();
            *shared_guard = stale_value;
            drop(shared_guard);
            drop(watch_guard);
        })
        .join()
        .expect("reader thread panicked");
    }

    {
        let tx1 = tx.clone();
        let watch = watch_state.clone();
        let shared = shared_state.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx1);
            set_probe_thread_id(1);

            let mut shared_guard = shared.lock();
            *shared_guard += 1;
            let mut watch_guard = watch.write();
            *watch_guard = *shared_guard;
            drop(watch_guard);
            drop(shared_guard);
        })
        .join()
        .expect("writer thread panicked");
    }

    drop(tx);
    clear_probe_sender();

    let events: Vec<ProbeEvent> = rx.into_iter().collect();
    println!("[S7] Collected {} events", events.len());

    let config = bug_config();
    run_verification_from(&events, "tokio_watch_mutex_race", &config).assert_bug();

    println!("[S7] PASS — watch+mutex check-then-act race model detected");
}
