# Laplace (Public SDK)

Apache-2.0 공개 크레이트. Ki-DPOR 엔진은 `laplace-cloud` 전용 — 이 레포에 없음.

## 규칙
- engine 크레이트(laplace-core, laplace-axiom, laplace-ki-dpor 등) 추가 금지
- Ki-DPOR 로직은 `laplace-cloud/crates/` 전용

## Active 크레이트 (crates.io 게시 대상)
- `laplace-interfaces` — ABI/FFI 타입 (#[repr(C)])
- `laplace-macro` — proc-macro (`#[laplace_sdk::verify]` 등)
- `laplace-probe-common` — RawProbeEvent 타입

## Inactive 크레이트 (alpha-2에서 private dep 분리 후 활성화)
- `laplace-sdk` — 사용자 진입점 re-export
- `laplace-probe-sdk` — TrackedMutex, BYOC 매크로

## 슬래시 커맨드
| 커맨드 | 용도 |
|--------|------|
| `/laca` | 설계/프롬프트 생성 (L-ACA 페르소나) |
| `/coder` | Rust 코딩 규칙 적용 |
