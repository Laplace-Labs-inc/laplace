// SPDX-License-Identifier: Apache-2.0
//! quilkin QCMP nonce waiter dispatch 프로토콜의 고객 소유 analog.
//!
//! Quilkin 코드를 복사하지 않고 계약만 재표현한다. response source가
//! `(nonce, payload)` 값을 공급하고, dispatcher 하나가 nonce lookup map을
//! 소유한 뒤 매칭된 payload를 단일 사용 waiter에 전달한다.

use std::collections::HashMap;

use laplace_sdk::rt::{mpsc, oneshot};

/// 이 analog에서 QCMP response가 전달하는 값.
pub type Response = (u64, u64);

/// 정상 프로토콜과 명시적 fault fixture를 고르는 정책.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchPolicy {
    /// 매칭된 모든 response를 nonce waiter에 전달한다.
    Normal,
    /// 테스트 전용 receive→lookup→deliver 규율의 역상.
    #[cfg(feature = "fault-fixture")]
    SuppressNonce(u64),
}

impl DispatchPolicy {
    fn suppresses(self, nonce: u64) -> bool {
        #[cfg(feature = "fault-fixture")]
        if let Self::SuppressNonce(suppressed) = self {
            return suppressed == nonce;
        }
        let _ = nonce;
        false
    }
}

/// response source가 닫힐 때까지 receive → lookup → oneshot deliver loop를
/// 실행한다. suppressed sender는 joiner에게 반환해 receiver를 pending으로
/// 유지한다. sender를 drop하면 lost-response liveness가 아니라 `RecvError`가
/// 발생한다.
pub async fn dispatch(
    mut source: mpsc::Receiver<Response>,
    mut waiters: HashMap<u64, oneshot::Sender<u64>>,
    handoff: oneshot::Sender<Vec<oneshot::Sender<u64>>>,
    policy: DispatchPolicy,
) {
    let mut retained = Vec::new();
    while let Some((nonce, payload)) = source.recv().await {
        if let Some(sender) = waiters.remove(&nonce) {
            if policy.suppresses(nonce) {
                retained.push(sender);
            } else {
                let _ = sender.send(payload);
            }
        }
    }
    let _ = handoff.send(retained);
}

/// 라우팅된 response 하나를 waiter 경계에서 await하고 payload를 검사한다.
pub async fn wait_for_response(receiver: oneshot::Receiver<u64>, expected: u64) {
    let payload = receiver.await.expect("matched nonce waiter was cancelled");
    assert_eq!(payload, expected);
}

/// Route A 공개 capture. 고객 채널 표기를 이 함수에 남겨
/// `verify(tasks)`가 `tokio::sync::{mpsc,oneshot}` rewrite 경로를 증명하게
/// 한다. Route B는 private async engine에서 같은 `dispatch`와 waiter 함수를
/// 조성한다.
#[allow(dead_code)]
#[laplace_sdk::verify(tasks, name = "quilkin_qcmp_nonce_lookup_deliver")]
fn quilkin_qcmp_nonce_lookup_deliver(tasks: &mut laplace_sdk::rt::TaskSet) {
    #[allow(unused_imports)]
    use tokio::sync::{mpsc, oneshot};

    let (source_tx, source_rx) = mpsc::channel(4);
    let (handoff, retained_rx) = oneshot::channel::<Vec<oneshot::Sender<u64>>>();
    let (first_tx, first_rx) = oneshot::channel();
    let (second_tx, second_rx) = oneshot::channel();

    let mut waiters = HashMap::new();
    waiters.insert(11, first_tx);
    waiters.insert(22, second_tx);

    let driver = tasks.spawn(async move {
        source_tx.send((11, 110)).await.expect("source is open");
        source_tx.send((22, 220)).await.expect("source is open");
    });
    let dispatcher = tasks.spawn(async move {
        dispatch(source_rx, waiters, handoff, DispatchPolicy::Normal).await;
    });
    let first_waiter = tasks.spawn(async move {
        wait_for_response(first_rx, 110).await;
    });
    let second_waiter = tasks.spawn(async move {
        wait_for_response(second_rx, 220).await;
    });
    tasks.spawn(async move {
        let retained = retained_rx.await.expect("dispatcher handoff is open");
        driver.await;
        dispatcher.await;
        first_waiter.await;
        second_waiter.await;
        drop(retained);
    });
}
