---
id: P1-QLK-QCMP
title: "Embark P1 — quilkin QCMP nonce waiter dispatch analog"
type: TrophyReport
author: Codex
date: 2026-07-17
status: illustrative-counterfactual
scope: local-only
---

# P1-QLK-QCMP — QCMP receive→nonce lookup→oneshot deliver

이 예제는 pinned `quilkin@fddd3426d4b81bd7c6159c6e6efd33c0eb1b9b9e`의 QCMP
response routing 규율을 고객-소유 코드로 재표현한다. Quilkin 프로덕션 버그나
실제 UDP 경로의 결함을 주장하지 않는다.

## 계약

- response source는 real UDP 대신 `tokio::sync::mpsc`의 `(nonce, payload)` 값이다.
- dispatcher가 `HashMap<nonce, oneshot::Sender>`를 단독 소유하고, 수신 nonce를
  lookup한 뒤 정확한 waiter에 한 번 전달한다.
- source mpsc가 닫히면 dispatcher가 종료한다. 이는 원본의
  `CancellationToken`을 재현하지 않는다.
- fault-fixture의 `SuppressNonce(22)`는 sender를 drop하지 않고 joiner에게
  보관시켜, 도착한 response에 대한 waiter가 `RecvError`가 아니라 실제 pending
  상태로 남도록 한다.

## Route A

`#[laplace_sdk::verify(tasks)]` 함수는 customer spelling
`tokio::sync::{mpsc, oneshot}`를 사용한다. 매크로 rewrite가 생성자·타입을
`::laplace_sdk::rt::{mpsc, oneshot}`로 바꾸며, native capture는 CLEAN이었다.

Route B와 동일한 5-task 조성은 driver 1, dispatcher 1, waiter 2, joiner 1이다.
`arc-swap`, broadcast, real I/O는 사용하지 않는다.

## 경계

이 analog가 주장하지 않는 것은 real UDP/socket I/O, packet loss/reordering,
DashMap 동시 변이, `CancellationToken` cancellation/select replay, timeout,
multi-thread runtime, endpoint 이동, tasks>8, 실제 Quilkin production bug다.
payload는 순수 `u64` 값이며 QCMP codec·nonce pool·atomics는 모델하지 않는다.
