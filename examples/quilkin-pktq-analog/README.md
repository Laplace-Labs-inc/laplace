---
id: P1-QLK-W
title: "Embark P1 — quilkin packet-queue push/notify/drain analog"
type: TrophyReport
author: Codex
date: 2026-07-16
status: implementation-in-progress
scope: local-only
---

# P1-QLK-W — quilkin 패킷 큐 통지 프로토콜 analog

이 디렉토리는 pinned `quilkin@fddd3426d4b81bd7c6159c6e6efd33c0eb1b9b9e`의
패킷 큐 규율을 고객-소유 코드로 재표현한다. Quilkin 프로덕션 버그나 실제
missing-notify 경로를 주장하지 않는다. 최종 판정은 Route B의 직접 엔진
결과와 생성된 트로피 번들만으로 기록한다.

## 원본 대응표

| pinned quilkin | analog | 대체·제외 |
|---|---|---|
| `src/net/packet/queue.rs:12,28,73` — `parking_lot::Mutex<Vec<SendPacket>>` | `src/lib.rs:22-25,40-47` — `ModelAsyncMutex<Vec<u64>>` | 동기 `parking_lot`을 Laplace 모델 락으로 대체. 짧은 임계구역이며 await를 가로지르지 않는다. 실제 `SendPacket` payload/destination은 제외. |
| `queue.rs:83-96` — push 후 `tx.send(true)` | `src/lib.rs:51-70` — `enqueue` 후 watch `send(true)` | `fault-fixture`에서 이 send만 생략하는 counterfactual을 별도 feature로 둔다. |
| `queue.rs:28` — `watch::channel(true)` | `src/lib.rs:40` — `rt::watch::channel(true)` | wrap-real watch seam의 `send`/`changed`만 사용. |
| `src/net/io/poll/tokio.rs:163,300` — `changed().await` 후 swap/drain | `src/lib.rs:79-107` — `changed().await` 후 모델 락 아래 drain | real socket I/O, poll backend, metrics, TTL, 플랫폼별 EventFd 분기는 주장하지 않는다. |

## Route와 판정

- Route A는 `#[laplace_sdk::verify(tasks)]`가 조성한 producer 1 + consumer 1 +
  shutdown 1, 총 3 tasks의 native capture다.
- Route B는 같은 `PacketQueue` 메서드를 `AsyncLiveSource`로 직접 조성한다.
  `fault-fixture`의 `SuppressOnePushWake`가 정상 enqueue의 실제 notification만
  생략한다. 손제작 event stream, `pending()`, `sleep`, panic, verdict 조작은 없다.
- 기본 기대 결과는 정상 analog `Clean`, feature-gated missing-notify
  counterfactual `BugFound` witness다. 이는
  `illustrative: quilkin@fddd342's packet queue holds a push→notify discipline;
  here is the violation Laplace proves hangs.` 라벨로만 보고한다.

## 경계 스코프 매니페스트

주장하는 표면은 단일 receiver watch `send`/`changed`, 모델 락 아래의 queue
push/drain, bounded task count뿐이다. 다음은 포함하지 않는다: `real-io`,
`multi-thread-runtime`, `thread-local-sharding`, `multi-receiver-mpsc`,
`channel-endpoint-move`, `tasks>8`, 실제 `SendPacket` payload, socket/poll,
EventFd, metrics, TTL, backpressure, starvation, watch `borrow` 자체의 hook
경계, `send_modify`/`send_replace`/`wait_for` 및 기타 wrapper 잔여 컷.

최종 번들, digest, seed, receipt, scrub 전후 ARD, pin/replay 검증 결과와
20/20 결정성 결과는 `docs/concepts/roadmap/task-plan/plan/launch/embark-p1-quilkin-pktq/`
에 생성 후 기록한다.
