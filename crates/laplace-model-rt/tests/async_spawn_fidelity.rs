// SPDX-License-Identifier: Apache-2.0
//
// `serial()`의 guard는 프로세스 전역 async-spawn hook/id 상태를 보호한다.
#![allow(clippy::await_holding_lock)]

//! `tokio::spawn`과 `laplace_model_rt::spawn_task`의 native/model fidelity gate.
//!
//! native 열은 current-thread Tokio에서 실행하고, model 열은 public
//! `AsyncSpawnHook`으로 model future와 control을 수집한 뒤 수동 executor로
//! 실행한다. 단순 passthrough 동작과 join/abort shadow는 같은 assertion을
//! 두 API에 적용하고,
//! 모델 열에서는 가능한 poll 선택을 전부 열거해 관측 순서의 집합을 비교한다.

use laplace_model_rt::{
    clear_async_spawn_hook, install_async_spawn_hook, AsyncSpawnHook, TaskControl, TaskControlState,
};
use std::collections::BTreeSet;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};

static TEST_GUARD: StdMutex<()> = StdMutex::new(());

fn serial() -> StdMutexGuard<'static, ()> {
    TEST_GUARD.lock().unwrap_or_else(PoisonError::into_inner)
}

fn poll_once<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    let mut context = Context::from_waker(Waker::noop());
    future.poll(&mut context)
}

fn discard_spawn<T>(value: T) {
    let _ = value;
}

struct YieldOnce {
    yielded: bool,
}

impl YieldOnce {
    const fn new() -> Self {
        Self { yielded: false }
    }
}

impl Future for YieldOnce {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if self.yielded {
            Poll::Ready(())
        } else {
            self.yielded = true;
            context.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn s1_fire_and_forget_side_effect_is_visible_after_completion_matches() {
    let _serial = serial();
    clear_async_spawn_hook();

    macro_rules! scenario {
        ($spawn:path) => {{
            let effect = Arc::new(AtomicBool::new(false));
            let worker_effect = Arc::clone(&effect);
            let (done_tx, done_rx) = tokio::sync::oneshot::channel();

            discard_spawn($spawn(async move {
                worker_effect.store(true, Ordering::SeqCst);
                done_tx.send(()).expect("completion receiver");
            }));

            assert!(
                !effect.load(Ordering::SeqCst),
                "spawn must not execute the future synchronously"
            );
            done_rx.await.expect("spawned side effect completion");
            assert!(effect.load(Ordering::SeqCst));
        }};
    }

    scenario!(tokio::spawn);
    scenario!(laplace_model_rt::spawn_task);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn s2_spawn_task_returns_before_the_future_is_polled_matches() {
    let _serial = serial();
    clear_async_spawn_hook();

    macro_rules! scenario {
        ($spawn:path) => {{
            let started = Arc::new(AtomicBool::new(false));
            let worker_started = Arc::clone(&started);
            let (done_tx, done_rx) = tokio::sync::oneshot::channel();

            discard_spawn($spawn(async move {
                worker_started.store(true, Ordering::SeqCst);
                done_tx.send(()).expect("completion receiver");
            }));

            assert!(!started.load(Ordering::SeqCst));
            tokio::task::yield_now().await;
            done_rx.await.expect("spawned task completion");
            assert!(started.load(Ordering::SeqCst));
        }};
    }

    scenario!(tokio::spawn);
    scenario!(laplace_model_rt::spawn_task);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn s3_spawned_panic_does_not_panic_the_parent() {
    let _serial = serial();
    clear_async_spawn_hook();

    let native = tokio::spawn(async { panic!("native child panic") });
    assert!(
        native.await.is_err(),
        "Tokio exposes the child panic in JoinError"
    );

    let started = Arc::new(AtomicBool::new(false));
    let worker_started = Arc::clone(&started);
    laplace_model_rt::spawn_task(async move {
        worker_started.store(true, Ordering::SeqCst);
        panic!("fire-and-forget child panic");
    });

    for _ in 0..8 {
        if started.load(Ordering::SeqCst) {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(started.load(Ordering::SeqCst));

    // native Tokio는 JoinError를 노출하지만 fire-and-forget API는 handle을
    // 버린다. 두 경우 모두 부모 task의 실행은 계속되어야 한다.

    let hook = Arc::new(CapturingSpawnHook::default());
    install_async_spawn_hook(hook.clone());
    laplace_model_rt::spawn_task(async { panic!("model child panic") });
    let mut model_futures = hook.take();
    clear_async_spawn_hook();
    assert_eq!(model_futures.len(), 1);
    assert!(catch_unwind(AssertUnwindSafe(|| {
        let _ = poll_once(model_futures[0].as_mut());
    }))
    .is_err());
}

type CapturedFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

struct CapturedControl;

impl TaskControl for CapturedControl {
    fn poll(&self, _cx: &mut Context<'_>) -> Poll<TaskControlState> {
        Poll::Pending
    }

    fn abort(&self) {}

    fn is_finished(&self) -> bool {
        false
    }
}

#[derive(Default)]
struct CapturingSpawnHook {
    futures: StdMutex<Vec<CapturedFuture>>,
}

impl CapturingSpawnHook {
    fn take(&self) -> Vec<CapturedFuture> {
        std::mem::take(&mut *self.futures.lock().expect("captured futures lock"))
    }
}

impl AsyncSpawnHook for CapturingSpawnHook {
    fn spawn_task(&self, future: CapturedFuture) -> Box<dyn TaskControl> {
        self.futures
            .lock()
            .expect("captured futures lock")
            .push(future);
        Box::new(CapturedControl)
    }
}

type Trace = Arc<StdMutex<Vec<u8>>>;

fn record(trace: &Trace, value: u8) {
    trace.lock().expect("trace lock").push(value);
}

async fn ordered_program(id: u8, trace: Trace) {
    record(&trace, id);
    YieldOnce::new().await;
    record(&trace, id + 10);
}

async fn native_order(task_count: u8) -> Vec<u8> {
    clear_async_spawn_hook();
    let trace = Arc::new(StdMutex::new(Vec::new()));
    let mut handles = Vec::new();
    for id in 0..task_count {
        handles.push(tokio::spawn(ordered_program(id, Arc::clone(&trace))));
    }
    for handle in handles {
        handle.await.expect("native ordered task");
    }
    Arc::try_unwrap(trace)
        .expect("native trace owners")
        .into_inner()
        .expect("native trace lock")
}

fn model_order(schedule: &[usize], task_count: u8) -> Vec<u8> {
    let hook = Arc::new(CapturingSpawnHook::default());
    install_async_spawn_hook(hook.clone());
    let trace = Arc::new(StdMutex::new(Vec::new()));
    for id in 0..task_count {
        laplace_model_rt::spawn_task(ordered_program(id, Arc::clone(&trace)));
    }
    let mut futures = hook.take();
    assert!(trace.lock().expect("trace lock").is_empty());
    clear_async_spawn_hook();

    assert_eq!(futures.len(), task_count as usize);
    for &task in schedule {
        let result = poll_once(futures[task].as_mut());
        assert!(
            matches!(result, Poll::Pending | Poll::Ready(())),
            "model task poll must not panic"
        );
    }
    drop(futures);

    Arc::try_unwrap(trace)
        .expect("model trace owners")
        .into_inner()
        .expect("model trace lock")
}

fn all_poll_schedules(states: &mut [u8], prefix: &mut Vec<usize>, output: &mut Vec<Vec<usize>>) {
    if states.iter().all(|state| *state == 2) {
        output.push(prefix.clone());
        return;
    }

    for task in 0..states.len() {
        if states[task] == 2 {
            continue;
        }
        states[task] += 1;
        prefix.push(task);
        all_poll_schedules(states, prefix, output);
        prefix.pop();
        states[task] -= 1;
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn s4_multiple_spawn_order_is_native_observation_in_model_full_exploration() {
    let _serial = serial();
    const TASK_COUNT: u8 = 3;

    let native = native_order(TASK_COUNT).await;
    let mut schedules = Vec::new();
    all_poll_schedules(
        &mut vec![0; TASK_COUNT as usize],
        &mut Vec::new(),
        &mut schedules,
    );

    let model_orders: BTreeSet<Vec<u8>> = schedules
        .iter()
        .map(|schedule| model_order(schedule, TASK_COUNT))
        .collect();

    assert!(
        model_orders.len() > 1,
        "모델은 여러 spawn poll 순서를 열어야 함"
    );
    assert!(
        model_orders.contains(&native),
        "native 관측 순서가 model 전수 탐색 집합에 없음: native={native:?}"
    );
    assert!(native.iter().all(|event| *event < 20));

    // MAX_ASYNC_THREADS=8과 동적 task-id(1<<63) 같은 속성은 엔진/캡처
    // 계층의 계약이므로 이 runtime-only fidelity 열에서는 비교하지 않는다.
    clear_async_spawn_hook();
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn j1_join_await_passes_the_child_output_through_both_native_surfaces() {
    let _serial = serial();
    clear_async_spawn_hook();

    let tokio_value = tokio::spawn(async { 7_u8 })
        .await
        .expect("tokio child must join");
    let shadow_value = laplace_model_rt::spawn_task(async { 7_u8 })
        .await
        .expect("shadow child must join");

    assert_eq!(tokio_value, shadow_value);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn j2_abort_resolves_as_cancelled_for_tokio_and_shadow() {
    let _serial = serial();
    clear_async_spawn_hook();

    let tokio_handle = tokio::spawn(std::future::pending::<()>());
    tokio_handle.abort();
    assert!(tokio_handle.await.is_err());

    let shadow = laplace_model_rt::spawn_task(std::future::pending::<()>());
    shadow.abort();
    assert!(shadow
        .await
        .expect_err("shadow task must cancel")
        .is_cancelled());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn j3_abort_after_completion_is_a_no_op() {
    let _serial = serial();
    clear_async_spawn_hook();

    let tokio_handle = tokio::spawn(async { 11_u8 });
    tokio::task::yield_now().await;
    assert!(tokio_handle.is_finished());
    tokio_handle.abort();
    assert_eq!(tokio_handle.await.expect("completed tokio child"), 11);

    let shadow = laplace_model_rt::spawn_task(async { 11_u8 });
    tokio::task::yield_now().await;
    assert!(shadow.is_finished());
    shadow.abort();
    assert_eq!(shadow.await.expect("completed shadow child"), 11);
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn j4_abort_before_first_poll_does_not_poll_either_child() {
    let _serial = serial();
    clear_async_spawn_hook();

    let tokio_polls = Arc::new(AtomicBool::new(false));
    let tokio_polls_for_child = Arc::clone(&tokio_polls);
    let tokio_handle = tokio::spawn(async move {
        tokio_polls_for_child.store(true, Ordering::SeqCst);
    });
    tokio_handle.abort();
    assert!(tokio_handle.await.is_err());
    assert!(!tokio_polls.load(Ordering::SeqCst));

    let shadow_polls = Arc::new(AtomicBool::new(false));
    let shadow_polls_for_child = Arc::clone(&shadow_polls);
    let shadow = laplace_model_rt::spawn_task(async move {
        shadow_polls_for_child.store(true, Ordering::SeqCst);
    });
    shadow.abort();
    assert!(shadow
        .await
        .expect_err("shadow child must cancel")
        .is_cancelled());
    assert!(!shadow_polls.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn j6_native_child_panic_is_exposed_as_panic_join_error() {
    let _serial = serial();
    clear_async_spawn_hook();

    let tokio_error = tokio::spawn(async { panic!("native fidelity panic") })
        .await
        .expect_err("tokio panic must be a JoinError");
    assert!(tokio_error.is_panic());

    let shadow_error = laplace_model_rt::spawn_task(async { panic!("shadow fidelity panic") })
        .await
        .expect_err("shadow panic must be a TaskJoinError");
    assert!(shadow_error.is_panic());
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn j7_dropping_either_handle_detaches_the_child() {
    let _serial = serial();
    clear_async_spawn_hook();

    let tokio_done = Arc::new(AtomicBool::new(false));
    let tokio_done_for_child = Arc::clone(&tokio_done);
    drop(tokio::spawn(async move {
        tokio_done_for_child.store(true, Ordering::SeqCst);
    }));
    tokio::task::yield_now().await;
    assert!(tokio_done.load(Ordering::SeqCst));

    let shadow_done = Arc::new(AtomicBool::new(false));
    let shadow_done_for_child = Arc::clone(&shadow_done);
    drop(laplace_model_rt::spawn_task(async move {
        shadow_done_for_child.store(true, Ordering::SeqCst);
    }));
    tokio::task::yield_now().await;
    assert!(shadow_done.load(Ordering::SeqCst));
}
