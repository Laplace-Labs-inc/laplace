// SPDX-License-Identifier: Apache-2.0
//! Route A: `verify(tasks)`로 bb8 0.9.1 실코드를 캡처하는 BB2 예제.
#![allow(dead_code, deprecated)]

use bb8_async_patched::{ManageConnection, Pool};
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

#[derive(Clone, Debug)]
struct MockManager {
    broken: Arc<AtomicBool>,
}

impl MockManager {
    fn new(broken: Arc<AtomicBool>) -> Self {
        Self { broken }
    }
}

impl ManageConnection for MockManager {
    type Connection = u64;
    type Error = Infallible;

    async fn connect(&self) -> Result<Self::Connection, Self::Error> {
        Ok(1)
    }

    async fn is_valid(&self, _conn: &mut Self::Connection) -> Result<(), Self::Error> {
        Ok(())
    }

    fn has_broken(&self, _conn: &mut Self::Connection) -> bool {
        self.broken.load(Ordering::SeqCst)
    }
}

struct NotifyOnPending<F> {
    future: Pin<Box<F>>,
    parked: Option<laplace_sdk::rt::oneshot::Sender<()>>,
}

impl<F> NotifyOnPending<F> {
    fn new(future: F, parked: laplace_sdk::rt::oneshot::Sender<()>) -> Self {
        Self {
            future: Box::pin(future),
            parked: Some(parked),
        }
    }
}

impl<F: Future> Future for NotifyOnPending<F> {
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let result = self.future.as_mut().poll(cx);
        if result.is_pending() {
            if let Some(parked) = self.parked.take() {
                let _ = parked.send(());
            }
        }
        result
    }
}

fn register_pool_task(
    tasks: &mut laplace_sdk::rt::TaskSet,
    manager: MockManager,
    owner_tx: laplace_sdk::rt::oneshot::Sender<Arc<Pool<MockManager>>>,
    waiter_tx: laplace_sdk::rt::oneshot::Sender<Arc<Pool<MockManager>>>,
) {
    tasks.spawn(async move {
        let pool = Pool::builder()
            .max_size(1)
            .min_idle(Some(1))
            .max_lifetime(None)
            .idle_timeout(None)
            .test_on_check_out(false)
            .connection_timeout(Duration::from_secs(30))
            .build(manager)
            .await
            .expect("bb8 build");
        let pool = Arc::new(pool);
        owner_tx
            .send(Arc::clone(&pool))
            .expect("owner pool receiver");
        waiter_tx.send(pool).expect("waiter pool receiver");
    });
}

/// 취소 없는 정상 반환 경로. 이 함수는 매크로가 생성한 native capture test가
/// 실행하며, `LAPLACE_VERIFY_EVENTS_DIR`가 설정되면 private tier-2 입력을 남긴다.
#[laplace_sdk::verify(
    tasks,
    name = "bb8_hunt2_w_return",
    expected = "clean",
    determinism = "fully_deterministic"
)]
fn bb8_w_return(tasks: &mut laplace_sdk::rt::TaskSet) {
    let (owner_tx, owner_rx) = laplace_sdk::rt::oneshot::channel();
    let (waiter_tx, waiter_rx) = laplace_sdk::rt::oneshot::channel();
    register_pool_task(
        tasks,
        MockManager::new(Arc::new(AtomicBool::new(false))),
        owner_tx,
        waiter_tx,
    );

    tasks.spawn(async move {
        let pool = owner_rx.await.expect("owner pool");
        let connection = pool.get().await.expect("owner connection");
        drop(connection);
    });

    tasks.spawn(async move {
        let pool = waiter_rx.await.expect("waiter pool");
        let connection = pool.get().await.expect("waiter connection");
        drop(connection);
    });
}

/// Broken 반환 경로. owner가 보유한 연결을 broken으로 표시한 뒤 반환하므로
/// bb8의 `spawn_replenishing_approvals` 동적 task가 native capture에 나타나야 한다.
#[laplace_sdk::verify(
    tasks,
    name = "bb8_hunt2_w_broken",
    expected = "clean",
    determinism = "fully_deterministic"
)]
fn bb8_w_broken(tasks: &mut laplace_sdk::rt::TaskSet) {
    let broken = Arc::new(AtomicBool::new(false));
    let (owner_tx, owner_rx) = laplace_sdk::rt::oneshot::channel();
    let (waiter_tx, waiter_rx) = laplace_sdk::rt::oneshot::channel();
    let (owner_holds_tx, owner_holds_rx) = laplace_sdk::rt::oneshot::channel();
    let (parked_tx, parked_rx) = laplace_sdk::rt::oneshot::channel();

    register_pool_task(
        tasks,
        MockManager::new(Arc::clone(&broken)),
        owner_tx,
        waiter_tx,
    );

    tasks.spawn(async move {
        let pool = owner_rx.await.expect("owner pool");
        let connection = pool.get().await.expect("owner connection");
        owner_holds_tx.send(()).expect("waiter start signal");
        parked_rx.await.expect("waiter parked signal");
        broken.store(true, Ordering::SeqCst);
        drop(connection);
    });

    tasks.spawn(async move {
        let pool = waiter_rx.await.expect("waiter pool");
        owner_holds_rx.await.expect("owner holds signal");
        let get = pool.get();
        let _ = NotifyOnPending::new(get, parked_tx).await;
    });
}

/// `Notify` waiter를 select loser로 취소하는 경로. native capture는 성공적으로
/// 끝나지만, linear tier-2 replay는 waiter drop을 재현할 수 없어 exit 2가 정답이다.
#[laplace_sdk::verify(
    tasks,
    name = "bb8_hunt2_w_cancel",
    expected = "clean",
    determinism = "fully_deterministic"
)]
fn bb8_w_cancel(tasks: &mut laplace_sdk::rt::TaskSet) {
    let (owner_tx, owner_rx) = laplace_sdk::rt::oneshot::channel();
    let (waiter_tx, waiter_rx) = laplace_sdk::rt::oneshot::channel();
    let (owner_holds_tx, owner_holds_rx) = laplace_sdk::rt::oneshot::channel();
    let (parked_tx, parked_rx) = laplace_sdk::rt::oneshot::channel();
    let (cancel_tx, cancel_rx) = laplace_sdk::rt::oneshot::channel();
    let (cancelled_tx, cancelled_rx) = laplace_sdk::rt::oneshot::channel();

    register_pool_task(
        tasks,
        MockManager::new(Arc::new(AtomicBool::new(false))),
        owner_tx,
        waiter_tx,
    );

    tasks.spawn(async move {
        let pool = owner_rx.await.expect("owner pool");
        let connection = pool.get().await.expect("owner connection");
        owner_holds_tx.send(()).expect("waiter start signal");
        parked_rx.await.expect("waiter parked signal");
        cancel_tx.send(()).expect("cancel signal");
        cancelled_rx.await.expect("cancelled signal");
        drop(connection);
    });

    tasks.spawn(async move {
        let pool = waiter_rx.await.expect("waiter pool");
        owner_holds_rx.await.expect("owner holds signal");
        let get = pool.get();
        let parked_get = NotifyOnPending::new(get, parked_tx);
        tokio::select! {
            biased;
            result = parked_get => {
                let _ = result;
            }
            _ = cancel_rx => {
                cancelled_tx.send(()).expect("cancel acknowledgement");
            }
        }
    });
}
