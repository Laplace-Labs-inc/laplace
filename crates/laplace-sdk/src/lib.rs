// SPDX-License-Identifier: Apache-2.0
#![deny(clippy::all, clippy::pedantic)]
#![allow(unexpected_cfgs)]

//! Laplace SDK — the single entry point for deterministic concurrency verification.
//!
//! Users can access the full Laplace verification ecosystem by adding one line
//! to `Cargo.toml`.
//!
//! # Quick start
//!
//! ```toml
//! [dev-dependencies]
//! laplace-sdk = { path = "path/to/crates/sdk/laplace-sdk" }
//! ```
//!
//! ```rust,ignore
//! use laplace_sdk::prelude::*;
//!
//! #[laplace_tracked]
//! pub struct MyService {
//!     #[track]
//!     cache: Mutex<HashMap<String, String>>,
//!     config: Config,
//! }
//!
//! #[laplace_sdk::verify(threads = 2)]
//! async fn test_concurrent_access(state: &MyService) {
//!     let mut cache = state.cache.lock().await;
//!     cache.insert("key".into(), "value".into());
//! }
//! ```

pub mod prelude;

// ── 매크로 재수출 ────────────────────────────────────────────────────────────

/// Attribute macro for automatic Tracked* type substitution.
///
/// Transforms `#[track]` fields from standard sync primitives to Tracked* equivalents.
pub use laplace_macro::laplace_tracked;

/// Struct macro for production observation (cloud Probe transmission enabled).
///
/// Performs the same type substitutions as `#[laplace_tracked]` and forwards
/// events when cloud observation is enabled.
pub use laplace_macro::laplace_probe;

/// Improved DPOR verification harness attribute.
///
/// Supports replica mode with `threads = N`, native one-shot scenario mode
/// with `scenario`, and pre-registered async task composition with `tasks`.
/// In scenario and tasks modes, a real `expected = "bug"` deadlock body can
/// hang during capture; use `laplace axiom verify --capture` as the
/// authoritative tier-2 bug reproduction path. `laplace_sdk::check!` is a
/// reserved invariant hook name and is not implemented yet.
pub use laplace_macro::laplace_verify as verify;

/// Annotates a model function and routes qualified `std::thread::spawn` calls
/// through the Laplace runtime spawn seam.
pub use laplace_macro::model;

/// Automated DPOR verification harness (legacy no-op public API).
///
/// Use `#[laplace_sdk::verify(...)]` for new code.
pub use laplace_macro::axiom_target;

/// Create an `Arc<TrackedMutex<T>>` with an optional resource name.
pub use laplace_macro::mutex;

/// Create an `Arc<TrackedRwLock<T>>` with an optional resource name.
pub use laplace_macro::rwlock;

/// Marker attribute for documentation purposes (zero runtime cost).
pub use laplace_macro::laplace_meta;

// ── Tracked 프리미티브 재수출 ────────────────────────────────────────────────

/// Async-based `Mutex` wrapper with automatic event tracking.
pub use laplace_probe_sdk::TrackedMutex;

/// Read guard for `TrackedMutex` (`Deref` only).
pub use laplace_probe_sdk::TrackedGuard;

/// Sync-based `Mutex` wrapper with automatic event tracking.
pub use laplace_probe_sdk::TrackedStdMutex;

/// Read guard for `TrackedStdMutex` (`Deref` only).
pub use laplace_probe_sdk::TrackedStdGuard;

/// Async-based `RwLock` wrapper with automatic event tracking.
pub use laplace_probe_sdk::TrackedRwLock;

/// Shared (read) guard for `TrackedRwLock` (`Deref` only).
pub use laplace_probe_sdk::TrackedRwLockReadGuard;

/// Exclusive (write) guard for `TrackedRwLock` (`Deref` + `DerefMut`).
pub use laplace_probe_sdk::TrackedRwLockWriteGuard;

/// Sync-based `RwLock` wrapper with automatic event tracking.
pub use laplace_probe_sdk::TrackedStdRwLock;

/// Shared (read) guard for `TrackedStdRwLock` (`Deref` only).
pub use laplace_probe_sdk::TrackedStdRwLockReadGuard;

/// Exclusive (write) guard for `TrackedStdRwLock` (`Deref` + `DerefMut`).
pub use laplace_probe_sdk::TrackedStdRwLockWriteGuard;

/// Atomic bool wrapper with `load`/`store`/`CAS` tracking.
pub use laplace_probe_sdk::TrackedAtomicBool;

/// Atomic u32 wrapper with `load`/`store`/`CAS`/`fetch_add`/`fetch_sub` tracking.
pub use laplace_probe_sdk::TrackedAtomicU32;

/// Atomic u64 wrapper with `load`/`store`/`CAS`/`fetch_add`/`fetch_sub` tracking.
pub use laplace_probe_sdk::TrackedAtomicU64;

/// Atomic usize wrapper with `load`/`store`/`CAS`/`fetch_add`/`fetch_sub` tracking.
pub use laplace_probe_sdk::TrackedAtomicUsize;

/// Semaphore wrapper with acquire/release event tracking.
pub use laplace_probe_sdk::TrackedSemaphore;

/// Permit guard for `TrackedSemaphore` (auto-release on drop).
pub use laplace_probe_sdk::TrackedSemaphorePermit;

// ── 검증 인프라 재수출 ────────────────────────────────────────────────────────

/// Set the probe event sender for the current thread.
pub use laplace_probe_sdk::set_probe_sender;

/// Clears the legacy per-thread and process-global probe sender.
pub use laplace_probe_sdk::clear_probe_sender;

/// Set the thread ID for probe event correlation.
pub use laplace_probe_sdk::set_probe_thread_id;

/// Configuration for DPOR verification sessions.
pub use laplace_probe_sdk::ProbeSessionConfig;

/// Enumeration of all probe event types.
pub use laplace_probe_sdk::{
    AsyncAcquireKind, AsyncChannelKind, AsyncChannelOp, AsyncChannelOutcome, AsyncChannelSide,
    ProbeEvent,
};

/// Dump captured events to `$LAPLACE_VERIFY_EVENTS_DIR` for the private CLI.
pub use laplace_probe_sdk::dump_events_if_configured;

/// Public reference verification result.
pub use laplace_probe_sdk::VerifyResult;

/// Public reference verifier verdict.
pub use laplace_probe_sdk::ReferenceVerdict;

/// Run the public reference verifier on a stream of probe events.
pub use laplace_probe_sdk::run_verification_from;

/// Scoped event-capture session used by generated `#[verify]` harnesses.
pub use laplace_probe_sdk::CaptureSession;

/// Runtime seams for annotated model code (`spawn`, `ModelMutex`, `ModelRwLock`,
/// un-modeled-primitive markers).
///
/// Generated macro output routes rewritten primitives through this single root
/// (`::laplace_sdk::rt::…`) so an adopter only ever depends on `laplace-sdk`;
/// the runtime crate is never a direct dependency of user code.
pub use laplace_rt as rt;

/// Project-level configuration loaded from laplace.toml.
pub use laplace_probe_sdk::ProjectConfig;

/// Load project configuration from laplace.toml.
pub use laplace_probe_sdk::load_project_config;

#[cfg(feature = "cloud")]
pub use laplace_probe_sdk::client::ProbeClientConfig;
/// Initializes cloud Probe observation.
///
/// Programmatically performs the same action as `probe agent start`.
#[cfg(feature = "cloud")]
pub use laplace_probe_sdk::init_cloud_probe;

/// Stable import surface for code generated by `laplace-macro`.
///
/// Macro expansions use this hidden module instead of individual root
/// re-exports. That keeps generated paths stable even if the facade moves
/// implementation details between product-specific SDK crates later.
#[doc(hidden)]
pub mod __macro_support {
    pub use crate::ProbeEvent;
    pub use crate::{
        clear_probe_sender, dump_events_if_configured, run_verification_from, set_probe_sender,
        set_probe_thread_id, CaptureSession, ProbeSessionConfig, TrackedAtomicBool,
        TrackedAtomicU32, TrackedAtomicU64, TrackedAtomicUsize, TrackedMutex, TrackedRwLock,
        TrackedSemaphore, TrackedStdMutex, TrackedStdRwLock,
    };
    pub use laplace_probe_sdk::{
        dump_events_with_mode, install_probe_async_hooks, install_probe_lock_hook,
        install_probe_task_hook, run_task_set_native,
    };
    pub use tokio;
}
