# Laplace Open Zone

이 디렉토리: Apache-2.0 공개 크레이트.

## 규칙
- `../closed/` 디렉토리 수정 금지
- Ki-DPOR A* 로직 (ki_state.rs, ki_scheduler.rs) 은 이 존에 없음 — 추가 금지
- oracle/engine.rs 는 비공개 — 이 존에서 참조만 가능 (feature gate 유지)

## 주요 크레이트
- laplace-dpor: Classic DPOR + VectorClock (공개 알고리즘)
- laplace-axiom: VerificationSession + simulation (oracle/engine은 engine feature 뒤에 숨겨짐)
- laplace-probe-sdk: TrackedMutex — BYOC 핵심 통합 인터페이스
