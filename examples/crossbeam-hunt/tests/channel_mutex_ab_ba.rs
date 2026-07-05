#![deny(clippy::all, clippy::pedantic)]
#![allow(clippy::doc_markdown)]

//! crossbeam-channel sender + external Mutex AB-BA (Novel)
//!
//! 실서비스 패턴:
//!   메시지 큐(crossbeam bounded) + 처리 상태 추적 Mutex 조합.
//!
//! Thread 0 (Producer):
//!   state_mutex.lock() [처리 중 표시] → channel.send() [내부 lock 포함]
//!
//! Thread 1 (Consumer):
//!   channel.recv() [내부 lock 포함] → state_mutex.lock() [완료 처리]
//!
//! 탐지 조건: channel 내부 Mutex가 TrackedStdMutex로 패치됨.
//! 기대: BugFound (state_mutex → channel_inner vs channel_inner → state_mutex)

use laplace_probe_sdk::{
    clear_probe_sender, run_verification_from, set_probe_sender, set_probe_thread_id, ProbeEvent,
    ProbeSessionConfig, TrackedStdMutex,
};
use std::sync::{mpsc, Arc};

fn bug_config() -> ProbeSessionConfig {
    ProbeSessionConfig {
        max_depth: 100_000,
        write_ard: true,
        output_dir: ".".to_string(),
    }
}

use crossbeam_channel::bounded;

#[test]
fn channel_mutex_ab_ba() {
    let (probe_tx, probe_rx) = mpsc::sync_channel::<ProbeEvent>(8192);
    let (sender, receiver) = bounded::<i64>(2);
    sender.try_send(0_i64).expect("prefill send failed");
    let sender = Arc::new(sender);
    let receiver = Arc::new(receiver);
    let state = Arc::new(TrackedStdMutex::new(0_u64, "state"));

    {
        let tx = probe_tx.clone();
        let sender = sender.clone();
        let state = state.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx);
            set_probe_thread_id(0);

            let mut state_guard = state.lock();
            *state_guard += 1;
            sender.try_send(1_i64).expect("producer send failed");
            drop(state_guard);
        })
        .join()
        .expect("producer thread panicked");
    }

    {
        let tx = probe_tx.clone();
        let receiver = receiver.clone();
        let state = state.clone();
        std::thread::spawn(move || {
            set_probe_sender(tx);
            set_probe_thread_id(1);

            let _message = receiver.try_recv().expect("consumer recv failed");
            let mut state_guard = state.lock();
            *state_guard += 1;
            drop(state_guard);
        })
        .join()
        .expect("consumer thread panicked");
    }

    drop(probe_tx);
    clear_probe_sender();

    let events: Vec<ProbeEvent> = probe_rx.into_iter().collect();
    assert!(
        !events.is_empty(),
        "crossbeam laplace feature emitted no events"
    );
    run_verification_from(&events, "channel_mutex_ab_ba", &bug_config()).assert_bug();
}
