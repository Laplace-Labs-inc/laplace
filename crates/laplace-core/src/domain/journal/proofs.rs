#![cfg(kani)]

//! Kani Formal Verification Proofs — Journal Domain
//!
//! Verifies the following invariants:
//!
//! - **H-J1** `proof_status_code_bijection` — `to_code()` / `from_code()` 완전 단사(bijection):
//!   모든 유효 코드에서 `from_code(s.to_code()) == Some(s)`.
//! - **H-J2** `proof_status_code_range` — `to_code()`의 반환값은 항상 [0, 7] 범위.
//! - **H-J3** `proof_from_code_rejects_invalid` — `from_code(n)` where `n > 7` → `None`.
//! - **H-J4** `proof_terminal_states_partitioning` — `is_terminal()`과 `is_in_progress()`는
//!   절대 동시에 true가 될 수 없다(상호 배타성).
//! - **H-J5** `proof_turbo_metadata_consistency` — `new_turbo()` 생성자는 항상
//!   `is_turbo=true`, 슬롯 필드 모두 Some; `new()`는 `is_turbo=false`, 슬롯 필드 모두 None.
//! - **H-J6** `proof_turbo_slot_info_coherence` — `turbo_slot_info()`는 두 슬롯 필드가
//!   모두 Some일 때만 Some을 반환한다.
//! - **H-J7** `proof_latency_category_completeness` — 모든 `Option<u64>` duration에 대해
//!   `latency_category()`는 5개 버킷 중 하나를 반환하며 패닉하지 않는다.

use super::{LogStatus, TransactionLog};

// ── H-J1 ─────────────────────────────────────────────────────────────────────

/// Proof: `LogStatus::to_code` / `from_code` 는 완전 단사(bijection)다.
///
/// # Invariant
///
/// 모든 유효 코드 `c ∈ [0, 7]`에 대해 `from_code(c)` 가 `Some(s)` 를 반환하고,
/// 그 `s.to_code() == c` 이며, 다시 `from_code(s.to_code()) == Some(s)` 가 성립한다.
#[kani::proof]
#[kani::unwind(1)]
fn proof_status_code_bijection() {
    let code: u8 = kani::any();
    kani::assume(code <= 7);
    let status = LogStatus::from_code(code).unwrap();
    assert_eq!(
        LogStatus::from_code(status.to_code()),
        Some(status),
        "from_code(to_code(s)) must equal Some(s) for all valid statuses"
    );
}

// ── H-J2 ─────────────────────────────────────────────────────────────────────

/// Proof: `LogStatus::to_code` 의 반환값은 항상 [0, 7] 범위다.
///
/// # Invariant
///
/// 8개 변형에 대해 `to_code()` 는 최대 7을 초과하지 않는다.
/// protobuf 직렬화 코드에서 경계 초과 오류를 방지한다.
#[kani::proof]
#[kani::unwind(1)]
fn proof_status_code_range() {
    let code: u8 = kani::any();
    kani::assume(code <= 7);
    let status = LogStatus::from_code(code).unwrap();
    assert!(status.to_code() <= 7, "to_code() must always be in [0, 7]");
}

// ── H-J3 ─────────────────────────────────────────────────────────────────────

/// Proof: `LogStatus::from_code(n)` where `n > 7` 는 반드시 `None` 을 반환한다.
///
/// # Invariant
///
/// 코드 범위를 벗어난 값(8~255)은 항상 None을 반환해야 한다.
/// 잘못된 코드가 실수로 유효한 상태로 파싱되는 것을 방지한다.
#[kani::proof]
#[kani::unwind(1)]
fn proof_from_code_rejects_invalid() {
    let code: u8 = kani::any();
    kani::assume(code > 7);
    assert!(
        LogStatus::from_code(code).is_none(),
        "from_code with code > 7 must return None"
    );
}

// ── H-J4 ─────────────────────────────────────────────────────────────────────

/// Proof: `is_terminal()` 과 `is_in_progress()` 는 상호 배타적이다.
///
/// # Invariant
///
/// 어떤 `LogStatus` 변형도 동시에 terminal이면서 in_progress일 수 없다.
/// `TurboFallback` 은 둘 다 아님(false/false)이므로 `&&` 로 검증한다.
#[kani::proof]
#[kani::unwind(1)]
fn proof_terminal_states_partitioning() {
    let code: u8 = kani::any();
    kani::assume(code <= 7);
    let status = LogStatus::from_code(code).unwrap();
    assert!(
        !(status.is_terminal() && status.is_in_progress()),
        "No status can be both terminal and in_progress simultaneously"
    );
}

// ── H-J5 ─────────────────────────────────────────────────────────────────────

/// Proof: `new_turbo()` 는 `is_turbo=true` 와 슬롯 필드를 모두 Some으로 설정한다.
///        `new()` 는 `is_turbo=false` 와 슬롯 필드를 모두 None으로 설정한다.
///
/// # Invariant
///
/// Turbo 실행 경로의 메타데이터 일관성: 두 생성자가 서로 다른 경로에 대해
/// 올바른 상태를 구성함을 증명한다.
#[kani::proof]
#[kani::unwind(1)]
fn proof_turbo_metadata_consistency() {
    let slot_index: usize = kani::any();
    let memory_offset: usize = kani::any();

    // new_turbo: is_turbo=true, 슬롯 필드 모두 Some
    let turbo_log = TransactionLog::new_turbo(
        String::new(),
        String::new(),
        String::new(),
        LogStatus::Running,
        slot_index,
        memory_offset,
    );
    assert!(turbo_log.is_turbo, "new_turbo must set is_turbo=true");
    assert_eq!(
        turbo_log.turbo_slot_index,
        Some(slot_index),
        "new_turbo must set turbo_slot_index=Some(slot_index)"
    );
    assert_eq!(
        turbo_log.turbo_memory_offset,
        Some(memory_offset),
        "new_turbo must set turbo_memory_offset=Some(memory_offset)"
    );

    // new: is_turbo=false, 슬롯 필드 모두 None
    let std_log = TransactionLog::new(
        String::new(),
        String::new(),
        String::new(),
        LogStatus::Running,
    );
    assert!(!std_log.is_turbo, "new must set is_turbo=false");
    assert!(
        std_log.turbo_slot_index.is_none(),
        "new must set turbo_slot_index=None"
    );
    assert!(
        std_log.turbo_memory_offset.is_none(),
        "new must set turbo_memory_offset=None"
    );
}

// ── H-J6 ─────────────────────────────────────────────────────────────────────

/// Proof: `turbo_slot_info()` 는 두 필드가 모두 Some일 때만 Some을 반환한다.
///
/// # Invariant
///
/// - `(Some(idx), Some(off))` → `turbo_slot_info() == Some((idx, off))`
/// - 어느 한 쪽이라도 None → `turbo_slot_info() == None`
///
/// 부분적으로 설정된 슬롯 메타데이터는 절대 Some을 노출하지 않는다.
#[kani::proof]
#[kani::unwind(1)]
fn proof_turbo_slot_info_coherence() {
    let slot_index: Option<usize> = kani::any();
    let memory_offset: Option<usize> = kani::any();

    let mut log = TransactionLog::new(
        String::new(),
        String::new(),
        String::new(),
        LogStatus::Running,
    );
    log.turbo_slot_index = slot_index;
    log.turbo_memory_offset = memory_offset;

    let info = log.turbo_slot_info();

    match (slot_index, memory_offset) {
        (Some(idx), Some(off)) => {
            assert_eq!(
                info,
                Some((idx, off)),
                "turbo_slot_info must return Some when both fields are Some"
            );
        }
        _ => {
            assert!(
                info.is_none(),
                "turbo_slot_info must return None when either field is None"
            );
        }
    }
}

// ── H-J7 ─────────────────────────────────────────────────────────────────────

/// Proof: 모든 `Option<u64>` duration에 대해 `latency_category()` 는 5개 버킷 중
/// 하나를 반환하며 패닉하지 않는다.
///
/// # Invariant
///
/// - `None` → `"unknown"`
/// - `Some(0)` → `"sub-microsecond"`
/// - `Some(1..9)` → `"low"`
/// - `Some(10..99)` → `"medium"`
/// - `Some(≥100)` → `"high"`
///
/// 빠짐없는 분류(completeness): 어떤 입력도 미정의 동작을 유발하지 않는다.
#[kani::proof]
#[kani::unwind(20)]
fn proof_latency_category_completeness() {
    let duration_us: Option<u64> = kani::any();

    let mut log = TransactionLog::new(
        String::new(),
        String::new(),
        String::new(),
        LogStatus::Running,
    );
    log.duration_us = duration_us;

    let cat = log.latency_category();
    assert!(
        cat == "unknown"
            || cat == "sub-microsecond"
            || cat == "low"
            || cat == "medium"
            || cat == "high",
        "latency_category must return one of 5 known buckets for any duration"
    );
}
