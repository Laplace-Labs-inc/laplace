// SPDX-License-Identifier: Apache-2.0
//! Compile-time markers for concurrency primitives that `#[laplace::model]`
//! recognizes but cannot model.
//!
//! Referencing one of these deprecated constants emits a deprecation warning
//! carrying an honest "not modeled" note at the annotated function. The
//! `#[laplace::model]`/`#[laplace::verify]` rewrite injects such a reference
//! when it sees an un-modeled primitive in annotated source.
//!
//! Day-1 non-negotiable: an un-modeled primitive must never silently pass as a
//! verified green. See `substrate-shim-strategy.md` §4.5.

/// Marker for an un-modeled `std::sync::Condvar`.
#[deprecated(
    note = "#[laplace::model]: `Condvar` is not modeled; the verifier cannot observe waits/notifications here — this path is a verification blind spot"
)]
#[doc(hidden)]
pub const CONDVAR: () = ();

/// Marker for an un-modeled `std::sync::atomic` type.
#[deprecated(
    note = "#[laplace::model]: atomics are not modeled; the engine has no memory model — atomic orderings/races here are a verification blind spot"
)]
#[doc(hidden)]
pub const ATOMIC: () = ();

/// Marker for an un-modeled `std::sync::mpsc` channel — or a `tokio::sync`
/// channel the rewrite could not resolve (a glob import, or a bare call
/// with no `use` import as evidence).
#[deprecated(
    note = "#[laplace::model]: `mpsc` channels are not modeled; blocking send/recv here is a verification blind spot — if this is `tokio::sync::mpsc`, add `use tokio::sync::mpsc;` (or `use tokio::sync::mpsc::channel;`) inside the annotated fn, or annotate an inline `mod { ... }` instead of the fn, so the alias resolves to the modeled seam"
)]
#[doc(hidden)]
pub const CHANNEL: () = ();

// `TOKIO_CHANNEL` (un-modeled `tokio::sync::broadcast`) was removed at BCAST
// G4 keep (LEP-0027): broadcast is modeled via [`crate::broadcast`] and the
// macro rewrites it like the rest of the `mpsc`/`oneshot`/`watch` family.

/// Marker for an un-modeled `tokio::spawn` (or `tokio::task::spawn*`) task.
#[deprecated(
    note = "#[laplace::model]: `tokio::spawn` is not yet under deterministic executor control (AXM2 executor scope); tasks spawned here are a verification blind spot"
)]
#[doc(hidden)]
pub const TOKIO_SPAWN: () = ();

/// Marker for an un-modeled `tokio::time` primitive (`Instant`, `sleep_until`,
/// `interval_at`, `timeout_at`, `pause`/`advance`/`resume`, or a custom
/// `MissedTickBehavior`). `sleep`/`timeout`/`interval` themselves are modeled
/// (AXM2 A2-4) — see `laplace_model_rt::time`.
#[deprecated(
    note = "#[laplace::model]: this `tokio::time` primitive is not modeled (AXM2 A2-4 residue); wall-clock/virtual-clock semantics here are a verification blind spot"
)]
#[doc(hidden)]
pub const TOKIO_TIME: () = ();

/// Marker for an un-modeled `arc_swap` snapshot or cache primitive.
#[deprecated(
    note = "#[laplace::model]: `arc_swap` is not modeled; lock-free snapshot loads/stores here are invisible to the verifier — stale-read windows around them are a verification blind spot (BCAST G4; evidence-only modeling is tracked as A-1)"
)]
#[doc(hidden)]
pub const ARC_SWAP: () = ();
