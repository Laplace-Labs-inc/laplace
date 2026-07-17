---
id: P1-QLK-XDS
title: "Embark P1 — quilkin xDS control-plane stream fan-out analog"
type: TrophyReport
author: Codex
date: 2026-07-17
status: illustrative-counterfactual
scope: local-only
---

# P1-QLK-XDS — subscribe→change-broadcast→per-client forward

이 예제는 pinned `quilkin@fddd3426d4b81bd7c6159c6e6efd33c0eb1b9b9e`의 xDS
control-plane fan-out 규율을 고객 소유 코드로 재표현한다. Quilkin 프로덕션
버그나 실제 tonic transport 결함을 주장하지 않는다.

## 계약

- 슬라이스는 1 client, 1 resource type, reconnect 제외다.
- client가 unbounded request mpsc로 subscription을 보내면 server forwarder가
  bounded response mpsc(capacity 2)를 소유하고 변경 broadcast를 per-client로
  전달한다.
- config mutator는 version 1, 2, 3을 broadcast(capacity 2)에 commit한다.
  수신측 `Lagged(missed)`는 회복 가능한 분기로 기록 후 계속하며 final version을
  전달한다.
- shutdown은 watch receiver의 `changed()`로 닫고, explicit joiner가 task 수명을
  정리한다.
- `fault-fixture`의 `SuppressLastForward`는 final version만 생략한다. sender
  clone을 joiner가 보유해 client가 channel close가 아니라 committed update
  대기로 남도록 한다.

## 경계

tonic/gRPC transport·stream I/O, real network, timeout/interval, select
cancellation replay, CancellationToken, JoinSet/abort_all, multi-thread runtime,
multi-client/resource/reconnect, production VersionMap/Mutex mutation,
tasks>8, endpoint movement, 실제 Quilkin production bug는 모델하지 않는다.

Route A는 `#[laplace_sdk::verify(tasks)]` native capture이고, Route B direct-engine
결과와 번들은 상위 [launch report](../../../docs/concepts/roadmap/task-plan/plan/launch/embark-p1-quilkin-xds/README.md)에 기록한다.
