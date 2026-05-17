//! eBPF Lock Hunt 교차 검증 — std::sync::Mutex 순수 AB-BA 패턴.
//!
//! Spin-barrier로 두 스레드를 동시에 출발시켜 Rust Mutex slow-path(futex) 진입을 강제한다.
//! `std::sync::Barrier`는 내부 Mutex+Condvar가 관측 noise를 만들므로 사용 금지.
//!
//! # 사용법
//!
//! ```bash
//! cargo build -p ebpf-lock-test
//! ./target/debug/ebpf-lock-test 2>/tmp/addr.log
//! # stderr(/tmp/addr.log) 에서 mutex_a/mutex_b 주소 확인
//! # lock-hunt 부착 후 Enter
//! ```

use std::hint::spin_loop;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const ITERS: usize = 1_000;
const TIMEOUT: Duration = Duration::from_secs(10);
const N_WORKERS: usize = 2;

/// Atomic-only rendezvous — futex syscall을 발생시키지 않는다.
/// 모든 워커가 도달할 때까지 spin-wait.
fn wait_for_all_ready(ready: &AtomicUsize) {
    ready.fetch_add(1, Ordering::AcqRel);
    while ready.load(Ordering::Acquire) < N_WORKERS {
        spin_loop();
    }
}

fn main() {
    println!("[ebpf-lock-test] PID: {}", std::process::id());

    let mutex_a: Arc<Mutex<i64>> = Arc::new(Mutex::new(0));
    let mutex_b: Arc<Mutex<i64>> = Arc::new(Mutex::new(0));

    // stderr — mutex Arc 주소 로깅 (lock-hunt uaddr 대조용)
    // futex field는 Arc::as_ptr 기준 ±32 bytes 내에 위치한다.
    eprintln!(
        "[ebpf-lock-test] mutex_a @ Arc: {:p}",
        Arc::as_ptr(&mutex_a)
    );
    eprintln!(
        "[ebpf-lock-test] mutex_b @ Arc: {:p}",
        Arc::as_ptr(&mutex_b)
    );

    println!("[ebpf-lock-test] lock-hunt 부착 후 Enter를 눌러 AB-BA 테스트를 시작하세요…");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap_or(0);

    // 두 스레드의 동시 출발을 보장하는 spin-barrier.
    let ready: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));

    // Thread 0: A → B ordering
    let a0 = Arc::clone(&mutex_a);
    let b0 = Arc::clone(&mutex_b);
    let r0 = Arc::clone(&ready);
    let h0 = std::thread::spawn(move || {
        wait_for_all_ready(&r0);
        for i in 0..ITERS {
            let ga = a0.lock().unwrap();
            std::thread::yield_now();
            let gb = b0.lock().unwrap();
            drop(gb);
            drop(ga);
            if i % 250 == 0 {
                println!("  [T0] iter {} done", i);
            }
        }
    });

    // Thread 1: B → A ordering — 역순 (AB-BA)
    let a1 = Arc::clone(&mutex_a);
    let b1 = Arc::clone(&mutex_b);
    let r1 = Arc::clone(&ready);
    let h1 = std::thread::spawn(move || {
        wait_for_all_ready(&r1);
        for i in 0..ITERS {
            let gb = b1.lock().unwrap();
            std::thread::yield_now();
            let ga = a1.lock().unwrap();
            drop(ga);
            drop(gb);
            if i % 250 == 0 {
                println!("  [T1] iter {} done", i);
            }
        }
    });

    // Timeout-aware join — 실제 deadlock 시 process::exit(1).
    let start = Instant::now();
    let mut h0_opt = Some(h0);
    let mut h1_opt = Some(h1);

    while (h0_opt.is_some() || h1_opt.is_some()) && start.elapsed() < TIMEOUT {
        if let Some(h) = h0_opt.take() {
            if h.is_finished() {
                let _ = h.join();
            } else {
                h0_opt = Some(h);
            }
        }
        if let Some(h) = h1_opt.take() {
            if h.is_finished() {
                let _ = h.join();
            } else {
                h1_opt = Some(h);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    if h0_opt.is_some() || h1_opt.is_some() {
        println!(
            "[ebpf-lock-test] ⚠  TIMEOUT ({} secs) — real deadlock. Exiting.",
            TIMEOUT.as_secs()
        );
        std::process::exit(1);
    }

    println!("[ebpf-lock-test] Done.");
}
