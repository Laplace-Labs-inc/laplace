// SPDX-License-Identifier: Apache-2.0
//! Deterministic-select gate: [`laplace_select`] and the flag that drives it.
//!
//! See [`crate::set_deterministic_select`] for the runtime gate this macro
//! reads.

/// Runtime-gated drop-in for `tokio::select!`.
///
/// - If the caller writes `biased;` explicitly, that choice is respected
///   unconditionally — [`laplace_select`] never overrides an explicit user
///   choice.
/// - Otherwise, branch polling order is decided by
///   [`crate::deterministic_select_enabled`] at expansion's call time: **off**
///   (the default) expands to plain `tokio::select! { ... }` — tokio's stock
///   random branch start, identical to a plain user build with no model run
///   attached; **on** (set by the engine for the duration of a model run)
///   expands to `tokio::select! { biased; ... }` — polling in declaration
///   order, so the deterministic engine can replay branch selection.
///
/// The `if`/`else` expansion below duplicates the branch tokens once per
/// arm; only one arm's `tokio::select!` instance is ever actually reached at
/// runtime, but both must independently type/borrow-check, since the
/// compiler cannot see that they are mutually exclusive. A branch that moves
/// a captured variable therefore appears to move it "twice" from the
/// compiler's perspective — `tests/select_determinism.rs` (S1c) proves this
/// still compiles (each `if`/`else` arm has its own borrow scope, so there is
/// no conflict for the same reason two arms of an ordinary `if`/`else` can
/// each move the same outer variable).
#[macro_export]
macro_rules! laplace_select {
    (biased; $($rest:tt)*) => { ::tokio::select! { biased; $($rest)* } };
    ($($rest:tt)*) => {
        if $crate::deterministic_select_enabled() {
            ::tokio::select! { biased; $($rest)* }
        } else {
            ::tokio::select! { $($rest)* }
        }
    };
}
