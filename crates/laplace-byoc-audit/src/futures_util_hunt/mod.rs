// SPDX-License-Identifier: Apache-2.0
//! futures-util Ki-DPOR 검증 — Harness + BYOC 비교 실험
//!
//! 타겟 버그: cancel(Future drop) + waiter slab 경로 starvation
//! 제외: RUSTSEC-2020-0059, RUSTSEC-2020-0062, issue #2133

#[cfg(feature = "laplace")]
pub mod harness;
#[cfg(feature = "laplace")]
pub mod registry;
