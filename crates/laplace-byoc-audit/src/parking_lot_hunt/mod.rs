// SPDX-License-Identifier: Apache-2.0
//! parking_lot Ki-DPOR 검증 — Harness + BYOC 비교 실험
//!
//! 타겟 버그: Condvar requeue + RwLock upgrade 교차 경로 (미공개)
//! 제외: Issue #212, #489, #518 (이미 공개됨)

#[cfg(feature = "laplace")]
pub mod harness;
#[cfg(feature = "laplace")]
pub mod registry;
