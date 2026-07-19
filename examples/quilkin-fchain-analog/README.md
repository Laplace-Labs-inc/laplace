---
id: P1-QLK-FC
title: "Embark P1 — quilkin FilterChain store/notify/reload analog"
type: TrophyReport
author: Codex
date: 2026-07-17
status: implementation-in-progress
scope: local-only
---

# P1-QLK-FC — quilkin FilterChain hot-reload analog

이 디렉토리는 pinned `quilkin@fddd3426d4b81bd7c6159c6e6efd33c0eb1b9b9e`의
FilterChain 상태 교체·변경 통지·소비자 재로드 규율을 고객-소유 코드로 재표현한다.
Quilkin 프로덕션 버그를 주장하지 않는다.

## 계약과 구현

- 상태는 `ModelArcSwap<FilterChainConfig>`으로 표현한다. Laplace 내부의
  `Versioned<T>`가 값과 버전을 하나의 실제 `ArcSwap` snapshot에 동봉한다.
- 통지는 `tokio::sync::broadcast` customer syntax를 Route A의
  `#[laplace_sdk::verify(tasks)]` 매크로에 통과시킨다. 매크로가 `channel`과
  `Receiver` 표면을 `laplace_sdk::rt::broadcast`로 rewrite한다.
- `arc_swap`은 매크로 rewrite 대상이 아니다. 예제는 `laplace-model-rt`의 `arc-swap`
  feature를 직접 활성화하고 `ModelArcSwap`을 명시적으로 사용한다.
- fault-fixture는 store 뒤 broadcast send만 생략하는 counterfactual이다.

## 원본 대응표 상태

지정된 원본 checkout `/tmp/quilkin-recon-20260716/quilkin`은 이 실행 환경에
없었고, 동일 SHA 재clone은 `github.com` DNS 차단으로 실패했다. 따라서 아래
원본 줄은 작업 요청과 기존 정찰 보고서에 기록된 후보 경로를 넘어서 새로
주장하지 않는다. 최종 번들에서 이 재검증 불가 경계를 유지한다.

| 원본(정찰 보고서 기록) | analog | 대체·제외 |
|---|---|---|
| `src/config/filter.rs:4-50` — `FilterChainConfig::store/subscribe` | `src/lib.rs:29-65` — `ModelArcSwap` store와 broadcast sender | 실제 Quilkin payload·ArcSwap 구현은 복사하지 않음. 원본 파일 직접 재열람은 환경 차단. |
| `src/service.rs:950-1000` — filter-mutator consumer loop | `src/lib.rs:67-87` — `recv().await` 후 `load_full()` 재로드 | 실제 service, runtime, filter mutation side effects는 제외. 원본 파일 직접 재열람은 환경 차단. |

이 표의 원본 칸은 실물 재확인 전까지 provenance claim이 아니라 정찰 입력이다.

## Route와 기대 판정

Route A는 mutator 1 + consumer 1 + shutdown/join 1, 총 3 tasks다. Route B는
동일 `FilterChain`을 `AsyncLiveSource`로 직접 조성한다. 정상 analog는 Clean과
`WakeOrigin::BroadcastSend`를, fault-fixture는 BugFound witness를 기대한다.
fault 결과 라벨은 다음과 같다.

> illustrative: quilkin@fddd342's FilterChain hot-reload holds a store→notify discipline; here is the violation Laplace proves leaves consumers on a stale configuration.

## 경계

이 analog가 주장하는 것은 단일 receiver broadcast의 `send`/`recv`,
`ModelArcSwap` evidence-only store/load, snapshot payload revision, bounded
three-task shutdown join뿐이다. `Lagged`는 보존하지만 이 예제의 capacity 8에서는
정상 경로가 이를 밟지 않는다. 실제 UDP/socket I/O, multi-thread runtime,
timer/select cancellation, multi-receiver topology, tasks>8, ArcSwap의 `rcu`·CAS·
custom strategy, 매크로의 arc_swap 자동 rewrite, 실제 Quilkin production bug는
주장하지 않는다.
