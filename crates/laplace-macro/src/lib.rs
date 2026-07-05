// SPDX-License-Identifier: Apache-2.0
#![deny(clippy::all, clippy::pedantic)]
// Proc-macro parser/emitter functions keep token contracts local for auditability.
#![allow(clippy::items_after_statements, clippy::too_many_lines)]

//! Laplace procedural macros.
//!
//! Provides attribute and derive macros for DPOR verification and
//! automatic Tracked* primitive instrumentation.

use proc_macro::TokenStream;

mod byoc_test;
mod convenience;
mod harness;
mod model;
mod target;
mod tracked_derive;
mod verify;

use syn::parse_macro_input;

/// Register a function as a verification harness via `inventory`.
///
/// The decorated function must have the signature:
/// `fn(ThreadId, usize) -> Option<(Operation, ResourceId)>`
///
/// The macro emits the original function unchanged, followed by an
/// `inventory::submit!` block that statically registers a
/// `laplace_harness::registry::HarnessConfig`.
///
/// # Example
///
/// ```rust,ignore
/// #[axiom_harness(name = "template", threads = 2, resources = 1,
///                 desc = "Test harness")]
/// pub fn op_provider(_thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
///     match pc {
///         0 => Some((Operation::Request, ResourceId::new(0))),
///         1 => Some((Operation::Release, ResourceId::new(0))),
///         _ => None,
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn axiom_harness(attr: TokenStream, item: TokenStream) -> TokenStream {
    harness::axiom_harness_impl(attr, item)
}

/// Marker attribute for documentation and metadata purposes.
///
/// This attribute has no runtime effect and is purely informational.
#[proc_macro_attribute]
pub fn laplace_meta(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Create `Arc<TrackedMutex<T>>` with an optional resource name.
#[proc_macro]
pub fn mutex(input: TokenStream) -> TokenStream {
    convenience::mutex_impl(input)
}

/// Create `Arc<TrackedRwLock<T>>` with an optional resource name.
#[proc_macro]
pub fn rwlock(input: TokenStream) -> TokenStream {
    convenience::rwlock_impl(input)
}

/// Automated DPOR verification harness attribute (legacy no-op public API).
///
/// Generates a test function that runs a closure with N concurrent OS threads,
/// collects probe events, then discards them at the public macro boundary.
/// Use `#[laplace_tracked]` plus `#[laplace_sdk::verify]` for new code.
///
/// # Signature Requirements
///
/// - Function must be `async fn <name>(state: Arc<T>)` where T: Default
/// - First parameter must be `Arc<T>` — extracted and initialized with `T::default()`
///
/// # Generated Test Name
///
/// `__laplace_axiom_<original_fn_name>`
///
/// # Example
///
/// ```rust,ignore
/// #[axiom_target(threads = 3)]
/// async fn verify_counter(state: Arc<AppState>) {
///     let mut g = state.counter.lock().await;
///     *g += 1;
/// }
/// ```
#[deprecated(
    since = "0.1.0-alpha-1",
    note = "collects then discards events (target.rs `let _ = (...)`); use #[laplace_tracked] + #[laplace_sdk::verify] — the two-tier gate"
)]
#[proc_macro_attribute]
pub fn axiom_target(attr: TokenStream, item: TokenStream) -> TokenStream {
    target::axiom_target_impl(attr, item)
}

/// Attribute macro for automatic Tracked* type substitution and Default impl generation.
///
/// Transforms fields with `#[track]` attributes from standard sync primitives
/// (`Mutex`, `RwLock`, `Atomic*`, `Semaphore`) to their `Tracked*` equivalents.
///
/// # Field Annotation
///
/// ```rust,ignore
/// #[laplace_tracked]
/// pub struct MyService {
///     #[track]
///     cache: Mutex<HashMap<String, String>>,
///
///     #[track(name = "custom_name")]
///     counter: Mutex<i64>,
///
///     config: AppConfig,  // no #[track] — uses T::default()
/// }
/// ```
#[proc_macro_attribute]
pub fn laplace_tracked(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as syn::ItemStruct);
    tracked_derive::expand_attribute(proc_macro2::TokenStream::from(attr), input)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// `#[laplace_tracked]`의 deprecated alias.
///
/// `#[laplace_tracked]`와 동일하게 `#[track]` 필드를 TrackedMutex/TrackedRwLock 등으로 변환한다.
/// 클라우드 Probe 관측 활성 시 이벤트가 `GLOBAL_PROBE_CLIENT`를 통해 probe-edge로 전송된다.
///
/// # 예시
///
/// ```rust,ignore
/// #[laplace_probe]
/// pub struct AccountService {
///     #[track]
///     balance: tokio::sync::Mutex<i64>,
/// }
/// ```
#[deprecated(note = "identical to #[laplace_tracked]; use that")]
#[proc_macro_attribute]
pub fn laplace_probe(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as syn::ItemStruct);
    tracked_derive::expand_attribute(proc_macro2::TokenStream::from(attr), input)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// Improved DPOR verification harness attribute.
///
/// Generates a test function that runs a closure with N concurrent OS threads,
/// collects probe events, and runs DPOR verification. Supports both `&T`
/// references and `Arc<T>`, with automatic state initialization and management.
///
/// # Single-annotation control layer
///
/// `#[laplace::verify]` self-contains the model rewrite: it applies the same
/// `std::thread::spawn` → `::laplace_rt::spawn` and `std::sync::Mutex` →
/// `::laplace_rt::ModelMutex` rewrite as `#[laplace::model]` to the function body
/// before emitting the harness. Users therefore need only this one attribute;
/// the separate `#[laplace::model]` attribute remains available for backward
/// compatibility. Note that the compile-time `[patch.crates-io]` redirection is
/// emitted by onboarding (`laplace init`), not by this macro.
///
/// # Signature Requirements
///
/// - `async fn <name>(state: &T)` — replica mode state reference (recommended)
/// - `async fn <name>(state: Arc<T>)` — replica mode state Arc (backward compatible)
/// - `async fn <name>()` / `fn <name>()` — no shared state
/// - `#[laplace::verify(scenario)] fn <name>()` — one native scenario execution
///
/// Where T must implement `Default`.
///
/// # Parameters
///
/// - `threads`: Replica mode with this many concurrent workers (≤ 8)
/// - `scenario`: Scenario mode; no state parameter, body owns all worker setup
/// - `expected` (default: "clean"): Expected verdict: "clean" or "bug"
/// - `write_ard` (default: false): Write ARD output
/// - `output_dir` (default: "."): Output directory path
/// - `buffer` (default: 8192): Event channel buffer size
/// - `max_depth` (default: None): Max DPOR exploration depth
///
/// # Example
///
/// ```rust,ignore
/// #[laplace::verify(threads = 2, expected = "clean")]
/// async fn test_cache(state: &AppState) {
///     let mut cache = state.cache.lock().await;
///     cache.insert("key".into(), "value".into());
/// }
///
/// #[laplace::verify(scenario, expected = "clean")]
/// fn test_scenario() {
///     let handle = std::thread::spawn(|| {});
///     handle.join().unwrap();
/// }
/// ```
#[proc_macro_attribute]
pub fn laplace_verify(attr: TokenStream, item: TokenStream) -> TokenStream {
    verify::laplace_verify_impl(attr, item)
}

/// Annotates a model function and routes qualified `std::thread::spawn` calls
/// through `laplace_rt::spawn`.
///
/// P-1 rewrites exactly these call paths:
/// `std::thread::spawn`, `::std::thread::spawn`, and `thread::spawn`.
/// Bare `spawn(...)` is intentionally not rewritten because token-only macro
/// expansion cannot prove it came from `std::thread::spawn`.
#[proc_macro_attribute]
pub fn model(attr: TokenStream, item: TokenStream) -> TokenStream {
    model::model_impl(attr, item)
}

/// BYOC(Bring-Your-Own-Code) 테스트 보일러플레이트 제거용 attribute.
///
/// 원본 테스트 함수 본문을 래핑해 probe 채널/검증 테일을 자동 주입한다.
#[proc_macro_attribute]
pub fn laplace_byoc_test(attr: TokenStream, item: TokenStream) -> TokenStream {
    byoc_test::laplace_byoc_test_impl(attr, item)
}
