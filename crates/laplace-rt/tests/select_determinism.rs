// SPDX-License-Identifier: Apache-2.0
//
// `serial()`'s `std::sync::MutexGuard` is deliberately held across the
// `.await` points below — process-wide *test* serialization of the
// deterministic-select flag, not application state; each test's
// `current_thread` runtime runs exactly one task (mirrors
// `tests/async_mutex_fidelity.rs`/`tests/async_time_fidelity.rs`'s identical
// allow + rationale).
#![allow(clippy::await_holding_lock)]

//! Determinism gate for [`laplace_rt::laplace_select`] (AXM2 A2-4).
//!
//! `laplace_select!` is a *runtime-gated* `tokio::select!` drop-in — see the
//! macro's own doc for the full contract. In short: a user-written `biased;`
//! is always respected; otherwise [`laplace_rt::deterministic_select_enabled`]
//! (off by default, flipped on by the engine for a model run) decides
//! between tokio's stock random branch polling and `biased;` declaration
//! order.

use std::sync::{Mutex as StdMutex, MutexGuard as StdMutexGuard, PoisonError};

use laplace_rt::{deterministic_select_enabled, laplace_select, set_deterministic_select};

/// Serializes every test in this file — [`set_deterministic_select`] is
/// process-wide global state.
static TEST_GUARD: StdMutex<()> = StdMutex::new(());

fn serial() -> StdMutexGuard<'static, ()> {
    TEST_GUARD.lock().unwrap_or_else(PoisonError::into_inner)
}

/// S1a: flag on → `laplace_select!` always polls the first declared branch,
/// 100 times over, on a case where both branches are immediately ready (the
/// only way branch order is observable at all).
#[tokio::test(flavor = "current_thread")]
async fn s1a_deterministic_flag_on_always_first_branch() {
    let _serial = serial();
    set_deterministic_select(true);

    for i in 0..100 {
        let branch = laplace_select! {
            v = async { 1_u8 } => v,
            v = async { 2_u8 } => v,
        };
        assert_eq!(
            branch, 1,
            "s1a: iteration {i}: flag on must always poll the first declared branch"
        );
    }

    set_deterministic_select(false);
}

/// S1b: flag off (the default) → passthrough compiles and runs; true
/// randomness can't be asserted from a test, only that unbiased
/// `tokio::select!` semantics are actually reachable (both branches show up
/// over many draws — a biased-only implementation would fail this).
#[tokio::test(flavor = "current_thread")]
async fn s1b_deterministic_flag_off_passthrough_runs() {
    let _serial = serial();
    assert!(
        !deterministic_select_enabled(),
        "s1b: flag must default to off"
    );

    let mut saw_first = false;
    let mut saw_second = false;
    for _ in 0..200 {
        let branch = laplace_select! {
            v = async { 1_u8 } => v,
            v = async { 2_u8 } => v,
        };
        match branch {
            1 => saw_first = true,
            2 => saw_second = true,
            other => panic!("s1b: unexpected branch value {other}"),
        }
        if saw_first && saw_second {
            break;
        }
    }
    assert!(
        saw_first && saw_second,
        "s1b: passthrough (unbiased) mode must reach both branches over many \
         draws — got first={saw_first} second={saw_second}"
    );
}

/// S1c: a user-written `biased;` literal is respected unconditionally (the
/// macro's first arm), independent of the flag.
#[tokio::test(flavor = "current_thread")]
async fn s1c_user_written_biased_is_respected() {
    let _serial = serial();
    assert!(!deterministic_select_enabled());

    for i in 0..20 {
        let branch = laplace_select! {
            biased;
            v = async { 1_u8 } => v,
            v = async { 2_u8 } => v,
        };
        assert_eq!(
            branch, 1,
            "s1c: iteration {i}: user-written `biased;` must poll top-to-bottom"
        );
    }
}

/// Compile proof for the macro's `if`/`else` token duplication (second arm,
/// no `biased;` written): a non-`Copy` value moved into a branch's async
/// expression must compile even though it appears, textually, once per
/// `if`/`else` arm — each arm is its own mutually exclusive scope, exactly
/// like an ordinary `if`/`else` that moves the same outer variable in each
/// branch. If this test's crate fails to *compile*, that is the failure —
/// the runtime assertion below is secondary.
#[tokio::test(flavor = "current_thread")]
async fn move_captured_value_compiles_under_dual_expansion() {
    let _serial = serial();

    let owned = String::from("moved");
    let branch = laplace_select! {
        v = async move { owned } => v,
        v = async { String::from("other") } => v,
    };
    assert!(branch == "moved" || branch == "other");
}

/// Grammar: an `else` branch and a per-branch `if` precondition pass through
/// unchanged.
#[tokio::test(flavor = "current_thread")]
async fn else_branch_and_if_guard_grammar_compiles_and_runs() {
    let _serial = serial();

    let enabled = false;
    let branch = laplace_select! {
        () = async {}, if enabled => "guarded",
        else => "else",
    };
    assert_eq!(
        branch, "else",
        "a disabled `if` guard must fall through to `else`"
    );
}

/// Grammar smoke: `laplace_select!` accepts the same branch grammar
/// (pattern/precondition/handler) as a plain `tokio::select!` baseline and
/// produces the same result under `biased;` — proving the macro is a
/// drop-in rewrite target, not a different surface.
#[tokio::test(flavor = "current_thread")]
async fn grammar_is_isomorphic_with_raw_tokio_select() {
    let _serial = serial();

    let raw = tokio::select! {
        biased;
        v = async { 1_u8 } => v,
        v = async { 2_u8 } => v,
    };
    let shadow = laplace_select! {
        biased;
        v = async { 1_u8 } => v,
        v = async { 2_u8 } => v,
    };
    assert_eq!(raw, shadow);
}
