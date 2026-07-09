// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeSet;

use proc_macro::TokenStream;
use quote::quote;
use syn::parse_macro_input;
use syn::visit_mut::{self, VisitMut};
use syn::{
    parse_quote, Expr, ExprCall, Ident, ItemFn, Macro, Path, PathArguments, PathSegment, Stmt,
    TypePath,
};

pub fn model_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = proc_macro2::TokenStream::from(attr);
    if !attr.is_empty() {
        return syn::Error::new_spanned(
            attr,
            "unknown argument for `#[laplace::model]`; P-1 only accepts an empty attribute",
        )
        .to_compile_error()
        .into();
    }

    let mut input = parse_macro_input!(item as ItemFn);
    apply_model_rewrite(&mut input);
    quote!(#input).into()
}

/// Applies the shared model rewrite (spawn/Mutex/RwLock routing) and injects
/// un-modeled-primitive markers, in one pass, into an annotated function.
///
/// `#[laplace::model]` and `#[laplace::verify]` share this so a single
/// attribute performs the full rewrite before any harness is emitted.
pub(crate) fn apply_model_rewrite(func: &mut ItemFn) {
    let mut rewrite = ModelRewrite::default();
    rewrite.visit_item_fn_mut(func);
    rewrite.inject_unmodeled_markers(func);
}

/// `std`-qualified concurrency primitive rewriter shared by `#[laplace::model]`
/// and `#[laplace::verify]`.
///
/// Rewrites qualified `std::thread::spawn` → `::laplace_sdk::rt::spawn`,
/// `std::sync::{Mutex,RwLock}` → `::laplace_sdk::rt::{ModelMutex,ModelRwLock}`,
/// and `tokio::sync::Mutex` → `::laplace_sdk::rt::ModelAsyncMutex`, and
/// records any recognized-but-un-modeled primitive (`Condvar`, `atomic`,
/// `mpsc`, and the un-modeled `tokio::sync` family — `RwLock`, `Semaphore`,
/// `Notify`, and the channel constructors) so a compile-time blind-spot
/// warning can be injected.
#[derive(Default)]
pub(crate) struct ModelRewrite {
    unmodeled: BTreeSet<Unmodeled>,
}

/// A concurrency primitive `#[laplace::model]` recognizes but cannot model.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Unmodeled {
    Condvar,
    Atomic,
    Channel,
    TokioChannel,
    TokioSpawn,
    TokioTime,
}

impl Unmodeled {
    /// The `::laplace_sdk::rt::unmodeled` marker constant for this primitive.
    fn marker_ident(self) -> Ident {
        let name = match self {
            Unmodeled::Condvar => "CONDVAR",
            Unmodeled::Atomic => "ATOMIC",
            Unmodeled::Channel => "CHANNEL",
            Unmodeled::TokioChannel => "TOKIO_CHANNEL",
            Unmodeled::TokioSpawn => "TOKIO_SPAWN",
            Unmodeled::TokioTime => "TOKIO_TIME",
        };
        Ident::new(name, proc_macro2::Span::call_site())
    }
}

impl ModelRewrite {
    /// Prepends a `let _ = ::laplace_rt::unmodeled::<MARKER>;` statement for each
    /// distinct un-modeled primitive seen, emitting an honest deprecation
    /// warning at the annotated function (anti-false-green, day-1 non-negotiable).
    fn inject_unmodeled_markers(&self, func: &mut ItemFn) {
        if self.unmodeled.is_empty() {
            return;
        }
        let mut markers: Vec<Stmt> = self
            .unmodeled
            .iter()
            .map(|primitive| {
                let marker = primitive.marker_ident();
                parse_quote!(let _ = ::laplace_sdk::rt::unmodeled::#marker;)
            })
            .collect();
        markers.append(&mut func.block.stmts);
        func.block.stmts = markers;
    }
}

impl VisitMut for ModelRewrite {
    fn visit_expr_call_mut(&mut self, node: &mut ExprCall) {
        visit_mut::visit_expr_call_mut(self, node);

        let Expr::Path(path) = node.func.as_mut() else {
            return;
        };

        if is_supported_spawn_path(&path.path) {
            path.path = parse_quote!(::laplace_sdk::rt::spawn);
        } else if let Some(rewritten) = rewrite_std_sync_constructor_path(&path.path) {
            path.path = rewritten;
        } else if let Some(rewritten) = rewrite_tokio_sync_constructor_path(&path.path) {
            path.path = rewritten;
        } else if let Some(rewritten) = rewrite_tokio_time_fn_path(&path.path) {
            path.path = rewritten;
        } else if let Some(primitive) = classify_unmodeled(&path.path) {
            self.unmodeled.insert(primitive);
        }
    }

    fn visit_type_path_mut(&mut self, node: &mut TypePath) {
        visit_mut::visit_type_path_mut(self, node);

        if let Some(rewritten) = rewrite_std_sync_type_path(&node.path) {
            node.path = rewritten;
        } else if let Some(rewritten) = rewrite_tokio_sync_type_path(&node.path) {
            node.path = rewritten;
        } else if let Some(rewritten) = rewrite_tokio_time_type_path(&node.path) {
            node.path = rewritten;
        } else if let Some(primitive) = classify_unmodeled(&node.path) {
            self.unmodeled.insert(primitive);
        }
    }

    /// Rewrites a `tokio::select!` invocation's macro path to
    /// [`laplace_rt::laplace_select`](crate) — the runtime-gated,
    /// biased-under-a-model-run drop-in (see that macro's own doc).
    ///
    /// Only the macro's *path* is visited/rewritten here — `syn`'s default
    /// `visit_macro_mut` does not descend into the macro's token body (it is
    /// an opaque `TokenStream`, not a parsed syntax tree), so any
    /// fully-qualified `tokio::sync::...`/`tokio::time::...` paths written
    /// *inside* a `select!` branch are not seen or rewritten by this pass.
    /// Bare unqualified `select!` is intentionally excluded, mirroring the
    /// bare-`spawn` exclusion above — too high a false-positive risk against
    /// an unrelated user macro of the same name.
    fn visit_macro_mut(&mut self, node: &mut Macro) {
        visit_mut::visit_macro_mut(self, node);

        if is_tokio_select_macro_path(&node.path) {
            node.path = parse_quote!(::laplace_sdk::rt::laplace_select);
        }
    }
}

fn is_supported_spawn_path(path: &Path) -> bool {
    let segments: Vec<_> = path
        .segments
        .iter()
        .map(|segment| (&segment.ident, &segment.arguments))
        .collect();

    let all_plain = segments
        .iter()
        .all(|(_, arguments)| matches!(arguments, PathArguments::None));
    if !all_plain {
        return false;
    }

    matches!(
        segments.as_slice(),
        [(std, _), (thread, _), (spawn, _)]
            if *std == "std" && *thread == "thread" && *spawn == "spawn"
    ) || matches!(
        segments.as_slice(),
        [(thread, _), (spawn, _)] if *thread == "thread" && *spawn == "spawn"
    )
}

/// The `::laplace_rt` model type name for a `std::sync` lock type, if supported.
fn model_target_for(ident: &Ident) -> Option<&'static str> {
    if ident == "Mutex" {
        Some("ModelMutex")
    } else if ident == "RwLock" {
        Some("ModelRwLock")
    } else {
        None
    }
}

/// Rewrites a `std::sync::{Mutex,RwLock}` *type* path to its `::laplace_rt`
/// model equivalent, preserving generic arguments.
fn rewrite_std_sync_type_path(path: &Path) -> Option<Path> {
    let segments: Vec<_> = path.segments.iter().collect();
    let [std, sync, ty] = segments.as_slice() else {
        return None;
    };
    if std.ident != "std" || sync.ident != "sync" {
        return None;
    }
    if !matches!(std.arguments, PathArguments::None)
        || !matches!(sync.arguments, PathArguments::None)
    {
        return None;
    }

    let target = model_target_for(&ty.ident)?;
    Some(model_path(target, ty.arguments.clone(), None))
}

/// Rewrites a `std::sync::{Mutex,RwLock}::new` *constructor* path.
fn rewrite_std_sync_constructor_path(path: &Path) -> Option<Path> {
    let segments: Vec<_> = path.segments.iter().collect();
    let [std, sync, ty, method] = segments.as_slice() else {
        return None;
    };
    if std.ident != "std" || sync.ident != "sync" {
        return None;
    }
    if !matches!(std.arguments, PathArguments::None)
        || !matches!(sync.arguments, PathArguments::None)
        || method.ident != "new"
        || !matches!(method.arguments, PathArguments::None)
    {
        return None;
    }

    let target = model_target_for(&ty.ident)?;
    Some(model_path(
        target,
        ty.arguments.clone(),
        Some((*method).clone()),
    ))
}

fn model_path(target: &str, arguments: PathArguments, method: Option<PathSegment>) -> Path {
    let ident = Ident::new(target, proc_macro2::Span::call_site());
    let mut path: Path = parse_quote!(::laplace_sdk::rt::#ident);
    path.segments
        .last_mut()
        .expect("model path has a segment")
        .arguments = arguments;
    if let Some(method) = method {
        path.segments.push(method);
    }
    path
}

/// The `::laplace_rt` model type name for a `tokio::sync` type, if
/// supported. `Mutex`/`RwLock`/`Semaphore`/`Notify` are all modeled as of
/// AXM2 A2-3 slice 2; the `tokio::sync` channel family remains
/// recognized-but-un-modeled via [`classify_tokio_sync_unmodeled`].
fn tokio_model_target_for(ident: &Ident) -> Option<&'static str> {
    if ident == "Mutex" {
        Some("ModelAsyncMutex")
    } else if ident == "RwLock" {
        Some("ModelAsyncRwLock")
    } else if ident == "Semaphore" {
        Some("ModelAsyncSemaphore")
    } else if ident == "Notify" {
        Some("ModelAsyncNotify")
    } else {
        None
    }
}

/// Rewrites a `tokio::sync::Mutex` *type* path to its `::laplace_rt` model
/// equivalent, preserving generic arguments.
fn rewrite_tokio_sync_type_path(path: &Path) -> Option<Path> {
    let segments: Vec<_> = path.segments.iter().collect();
    let [tokio, sync, ty] = segments.as_slice() else {
        return None;
    };
    if tokio.ident != "tokio" || sync.ident != "sync" {
        return None;
    }
    if !matches!(tokio.arguments, PathArguments::None)
        || !matches!(sync.arguments, PathArguments::None)
    {
        return None;
    }

    let target = tokio_model_target_for(&ty.ident)?;
    Some(model_path(target, ty.arguments.clone(), None))
}

/// Rewrites a `tokio::sync::{Mutex,RwLock,Semaphore,Notify}::new` or
/// `::const_new` *constructor* path.
///
/// Unlike the `std::sync` constructor rewriter, `const_new` is accepted here
/// too — every tokio-side model type provides it (mirroring the real
/// `tokio::sync` types), whereas `ModelMutex`/`ModelRwLock` (`std::sync`
/// side) do not.
fn rewrite_tokio_sync_constructor_path(path: &Path) -> Option<Path> {
    let segments: Vec<_> = path.segments.iter().collect();
    let [tokio, sync, ty, method] = segments.as_slice() else {
        return None;
    };
    if tokio.ident != "tokio" || sync.ident != "sync" {
        return None;
    }
    if !matches!(tokio.arguments, PathArguments::None)
        || !matches!(sync.arguments, PathArguments::None)
        || !matches!(method.ident.to_string().as_str(), "new" | "const_new")
        || !matches!(method.arguments, PathArguments::None)
    {
        return None;
    }

    let target = tokio_model_target_for(&ty.ident)?;
    Some(model_path(
        target,
        ty.arguments.clone(),
        Some((*method).clone()),
    ))
}

/// The `::laplace_rt::time` free function name for a `tokio::time` seam
/// function, if supported. `sleep`/`timeout`/`interval` are modeled as of
/// AXM2 A2-4; the rest of `tokio::time` remains recognized-but-un-modeled
/// via [`classify_tokio_time_unmodeled`].
fn time_fn_target_for(ident: &Ident) -> Option<&'static str> {
    if ident == "sleep" {
        Some("sleep")
    } else if ident == "timeout" {
        Some("timeout")
    } else if ident == "interval" {
        Some("interval")
    } else {
        None
    }
}

/// Rewrites a `tokio::time::{sleep,timeout,interval}` *call* path to its
/// `::laplace_sdk::rt::time` equivalent.
fn rewrite_tokio_time_fn_path(path: &Path) -> Option<Path> {
    let segments: Vec<_> = path.segments.iter().collect();
    let [tokio, time, func] = segments.as_slice() else {
        return None;
    };
    if tokio.ident != "tokio" || time.ident != "time" {
        return None;
    }
    if !matches!(tokio.arguments, PathArguments::None)
        || !matches!(time.arguments, PathArguments::None)
        || !matches!(func.arguments, PathArguments::None)
    {
        return None;
    }

    let target = time_fn_target_for(&func.ident)?;
    let ident = Ident::new(target, proc_macro2::Span::call_site());
    Some(parse_quote!(::laplace_sdk::rt::time::#ident))
}

/// The `::laplace_rt::time` model type name for a `tokio::time` type, if
/// supported.
fn time_type_target_for(ident: &Ident) -> Option<&'static str> {
    if ident == "Sleep" {
        Some("Sleep")
    } else if ident == "Timeout" {
        Some("Timeout")
    } else if ident == "Interval" {
        Some("Interval")
    } else {
        None
    }
}

/// Rewrites a `tokio::time::{Sleep,Timeout,Interval}` *type* path to its
/// `::laplace_sdk::rt::time` equivalent, preserving generic arguments
/// (`Timeout<F>`'s `F`).
fn rewrite_tokio_time_type_path(path: &Path) -> Option<Path> {
    let segments: Vec<_> = path.segments.iter().collect();
    let [tokio, time, ty] = segments.as_slice() else {
        return None;
    };
    if tokio.ident != "tokio" || time.ident != "time" {
        return None;
    }
    if !matches!(tokio.arguments, PathArguments::None)
        || !matches!(time.arguments, PathArguments::None)
    {
        return None;
    }

    let target = time_type_target_for(&ty.ident)?;
    let ident = Ident::new(target, proc_macro2::Span::call_site());
    let mut rewritten: Path = parse_quote!(::laplace_sdk::rt::time::#ident);
    rewritten
        .segments
        .last_mut()
        .expect("time model path has a segment")
        .arguments = ty.arguments.clone();
    Some(rewritten)
}

/// Whether `path` is a supported `tokio::select!` macro path, by exact
/// plain-segment match. Unqualified bare `select!` is intentionally
/// excluded — see [`ModelRewrite::visit_macro_mut`].
fn is_tokio_select_macro_path(path: &Path) -> bool {
    let segments: Vec<_> = path
        .segments
        .iter()
        .map(|segment| (&segment.ident, &segment.arguments))
        .collect();

    let all_plain = segments
        .iter()
        .all(|(_, arguments)| matches!(arguments, PathArguments::None));
    if !all_plain {
        return false;
    }

    matches!(
        segments.as_slice(),
        [(tokio, _), (select, _)] if *tokio == "tokio" && *select == "select"
    )
}

/// Classifies a recognized-but-un-modeled `tokio::time::X` primitive by its
/// first three path segments (`tokio::time::X`, ignoring any trailing method
/// segment such as `::now`). `sleep`/`timeout`/`interval` and their model
/// types are excluded here — handled by the rewriters above. `Duration` and
/// the `error` module are excluded too: they are plain value/error types
/// this seam neither models nor treats as a blind spot.
fn classify_tokio_time_unmodeled(path: &Path) -> Option<Unmodeled> {
    let segments: Vec<_> = path.segments.iter().take(3).collect();
    let [tokio, time, ident] = segments.as_slice() else {
        return None;
    };
    if tokio.ident != "tokio" || time.ident != "time" {
        return None;
    }
    if !matches!(tokio.arguments, PathArguments::None)
        || !matches!(time.arguments, PathArguments::None)
    {
        return None;
    }

    if matches!(
        ident.ident.to_string().as_str(),
        "Instant" | "sleep_until" | "interval_at" | "timeout_at" | "advance" | "pause" | "resume"
    ) {
        Some(Unmodeled::TokioTime)
    } else {
        None
    }
}

/// Classifies a recognized-but-un-modeled `tokio::sync::X` primitive by its
/// first three path segments (`tokio::sync::X`, ignoring any trailing method
/// segment such as `::new`/`::channel`). `Mutex`/`RwLock`/`Semaphore`/
/// `Notify` are excluded here — all four are modeled and handled by
/// [`rewrite_tokio_sync_type_path`] / [`rewrite_tokio_sync_constructor_path`],
/// which run before this classifier in [`ModelRewrite`]'s visitor methods.
/// Only the `tokio::sync` channel family remains un-modeled.
fn classify_tokio_sync_unmodeled(path: &Path) -> Option<Unmodeled> {
    let segments: Vec<_> = path.segments.iter().take(3).collect();
    let [tokio, sync, ty] = segments.as_slice() else {
        return None;
    };
    if tokio.ident != "tokio" || sync.ident != "sync" {
        return None;
    }
    if !matches!(tokio.arguments, PathArguments::None)
        || !matches!(sync.arguments, PathArguments::None)
    {
        return None;
    }

    if matches!(
        ty.ident.to_string().as_str(),
        "mpsc" | "oneshot" | "watch" | "broadcast"
    ) {
        Some(Unmodeled::TokioChannel)
    } else {
        None
    }
}

/// Whether `path` is a supported `tokio::spawn`/`tokio::task::spawn*` call,
/// by exact plain-segment match. Unqualified bare `spawn(...)` is
/// intentionally excluded — too high a false-positive risk against an
/// unrelated user function of the same name.
fn is_unmodeled_tokio_spawn_path(path: &Path) -> bool {
    let segments: Vec<_> = path
        .segments
        .iter()
        .map(|segment| (&segment.ident, &segment.arguments))
        .collect();

    let all_plain = segments
        .iter()
        .all(|(_, arguments)| matches!(arguments, PathArguments::None));
    if !all_plain {
        return false;
    }

    matches!(
        segments.as_slice(),
        [(tokio, _), (spawn, _)] if *tokio == "tokio" && *spawn == "spawn"
    ) || matches!(
        segments.as_slice(),
        [(tokio, _), (task, _), (method, _)]
            if *tokio == "tokio"
                && *task == "task"
                && matches!(
                    method.to_string().as_str(),
                    "spawn" | "spawn_blocking" | "spawn_local"
                )
    )
}

/// Classifies a recognized-but-un-modeled concurrency primitive by its path
/// segments. Modeled primitives (`Mutex`/`RwLock`/`spawn`/
/// `tokio::sync::{Mutex,RwLock,Semaphore,Notify}`) return `None` and are
/// handled by the rewriters above.
fn classify_unmodeled(path: &Path) -> Option<Unmodeled> {
    if let Some(primitive) = classify_tokio_sync_unmodeled(path) {
        return Some(primitive);
    }
    if let Some(primitive) = classify_tokio_time_unmodeled(path) {
        return Some(primitive);
    }
    if is_unmodeled_tokio_spawn_path(path) {
        return Some(Unmodeled::TokioSpawn);
    }

    let has = |name: &str| path.segments.iter().any(|segment| segment.ident == name);

    if has("Condvar") {
        return Some(Unmodeled::Condvar);
    }
    if has("mpsc") {
        return Some(Unmodeled::Channel);
    }
    // Atomics: a genuine `std::sync::atomic` module segment, or a *known* std
    // atomic type name. Restricting the prefix match to the concrete atomic
    // types avoids false-positives on unrelated user types that merely start
    // with "Atomic" (e.g. `AtomicState`, `AtomicWaker`) — those must not inject
    // a blind-spot marker or the honest scope report lies about a clean run.
    let is_atomic = path.segments.iter().any(|segment| {
        let ident = segment.ident.to_string();
        ident == "atomic" || is_known_atomic_type(&ident)
    });
    if is_atomic {
        return Some(Unmodeled::Atomic);
    }
    None
}

/// Whether `ident` is one of the concrete `std::sync::atomic` types.
fn is_known_atomic_type(ident: &str) -> bool {
    matches!(
        ident,
        "AtomicBool"
            | "AtomicI8"
            | "AtomicI16"
            | "AtomicI32"
            | "AtomicI64"
            | "AtomicIsize"
            | "AtomicU8"
            | "AtomicU16"
            | "AtomicU32"
            | "AtomicU64"
            | "AtomicUsize"
            | "AtomicPtr"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::ToTokens;

    fn classify(path: &str) -> Option<Unmodeled> {
        let path: Path = syn::parse_str(path).expect("valid path");
        classify_unmodeled(&path)
    }

    /// Rewrites the annotated function and returns the emitted source as a
    /// whitespace-normalized string, so assertions are stable across rustc
    /// versions (unlike trybuild `.stderr` byte-matching).
    fn rewrite_to_string(func: &str) -> String {
        let mut func: ItemFn = syn::parse_str(func).expect("valid fn");
        apply_model_rewrite(&mut func);
        func.into_token_stream().to_string()
    }

    #[test]
    fn classify_flags_unmodeled_and_ignores_modeled() {
        assert_eq!(classify("std::sync::Condvar"), Some(Unmodeled::Condvar));
        assert_eq!(classify("Condvar"), Some(Unmodeled::Condvar));
        assert_eq!(
            classify("std::sync::mpsc::channel"),
            Some(Unmodeled::Channel)
        );
        assert_eq!(
            classify("std::sync::atomic::AtomicUsize"),
            Some(Unmodeled::Atomic)
        );
        assert_eq!(classify("AtomicBool"), Some(Unmodeled::Atomic));
        assert_eq!(classify("AtomicU64"), Some(Unmodeled::Atomic));
        // Modeled or unrelated paths must not be flagged.
        assert_eq!(classify("std::sync::Mutex"), None);
        assert_eq!(classify("std::sync::RwLock"), None);
        assert_eq!(classify("std::sync::Arc"), None);
        // "Atomic" alone is too generic to flag (avoids user-type false positives).
        assert_eq!(classify("Atomic"), None);
        // User types that merely start with "Atomic" are NOT std atomics and must
        // not be flagged (else a clean run injects a false blind-spot marker).
        assert_eq!(classify("AtomicState"), None);
        assert_eq!(classify("AtomicWaker"), None);
    }

    #[test]
    fn rewrite_maps_qualified_mutex_and_rwlock() {
        let out = rewrite_to_string(
            "fn f() { let a = std::sync::Mutex::new(0); let b = std::sync::RwLock::new(0); }",
        );
        assert!(out.contains("ModelMutex"), "mutex rewrite missing: {out}");
        assert!(out.contains("ModelRwLock"), "rwlock rewrite missing: {out}");
        // No un-modeled primitive → no blind-spot marker injected.
        assert!(!out.contains("unmodeled"), "unexpected marker: {out}");
    }

    #[test]
    fn rewrite_injects_blind_spot_marker_for_unmodeled_condvar() {
        let out = rewrite_to_string("fn f() { let c = std::sync::Condvar::new(); }");
        assert!(
            out.contains("laplace_sdk") && out.contains("unmodeled") && out.contains("CONDVAR"),
            "condvar blind-spot marker missing: {out}"
        );
    }

    #[test]
    fn rewrite_injects_one_marker_per_distinct_unmodeled_primitive() {
        let out = rewrite_to_string(
            "fn f() { let c1 = std::sync::Condvar::new(); let c2 = std::sync::Condvar::new(); }",
        );
        // Deduplicated via the BTreeSet: exactly one CONDVAR marker.
        assert_eq!(
            out.matches("CONDVAR").count(),
            1,
            "expected one marker: {out}"
        );
    }

    #[test]
    fn rewrite_maps_qualified_tokio_sync_mutex() {
        let out = rewrite_to_string(
            "fn f() { let m: tokio::sync::Mutex<u8> = tokio::sync::Mutex::new(0); }",
        );
        assert!(
            out.contains("ModelAsyncMutex"),
            "tokio mutex rewrite missing: {out}"
        );
        assert!(!out.contains("unmodeled"), "unexpected marker: {out}");
    }

    #[test]
    fn rewrite_maps_qualified_tokio_sync_rwlock_semaphore_notify() {
        let out = rewrite_to_string(
            "fn f() { \
                let r: tokio::sync::RwLock<u8> = tokio::sync::RwLock::new(0); \
                let s: tokio::sync::Semaphore = tokio::sync::Semaphore::new(1); \
                let n: tokio::sync::Notify = tokio::sync::Notify::new(); \
            }",
        );
        assert!(
            out.contains("ModelAsyncRwLock"),
            "tokio rwlock rewrite missing: {out}"
        );
        assert!(
            out.contains("ModelAsyncSemaphore"),
            "tokio semaphore rewrite missing: {out}"
        );
        assert!(
            out.contains("ModelAsyncNotify"),
            "tokio notify rewrite missing: {out}"
        );
        // All four tokio::sync lock-family primitives are modeled now — no
        // blind-spot marker injected.
        assert!(!out.contains("unmodeled"), "unexpected marker: {out}");
    }

    #[test]
    fn rewrite_maps_qualified_tokio_sync_const_new_constructors() {
        let out = rewrite_to_string(
            "fn f() { \
                static M: tokio::sync::Mutex<u8> = tokio::sync::Mutex::const_new(0); \
                static R: tokio::sync::RwLock<u8> = tokio::sync::RwLock::const_new(0); \
                static S: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1); \
                static N: tokio::sync::Notify = tokio::sync::Notify::const_new(); \
            }",
        );
        assert!(
            out.contains("ModelAsyncMutex") && out.contains("const_new"),
            "tokio mutex const_new rewrite missing: {out}"
        );
        assert!(
            out.contains("ModelAsyncRwLock"),
            "tokio rwlock const_new rewrite missing: {out}"
        );
        assert!(
            out.contains("ModelAsyncSemaphore"),
            "tokio semaphore const_new rewrite missing: {out}"
        );
        assert!(
            out.contains("ModelAsyncNotify"),
            "tokio notify const_new rewrite missing: {out}"
        );
        assert!(!out.contains("unmodeled"), "unexpected marker: {out}");
    }

    #[test]
    fn rewrite_does_not_mark_tokio_sync_mutex_family_as_unmodeled() {
        let out = rewrite_to_string(
            "fn f() { \
                let m = tokio::sync::Mutex::new(0); \
                let r = tokio::sync::RwLock::new(0); \
                let s = tokio::sync::Semaphore::new(1); \
                let n = tokio::sync::Notify::new(); \
            }",
        );
        assert!(
            !out.contains("TOKIO_") && !out.contains("unmodeled"),
            "tokio::sync lock-family types must not be flagged as un-modeled: {out}"
        );
    }

    #[test]
    fn rewrite_injects_blind_spot_marker_for_unmodeled_tokio_channel() {
        let out = rewrite_to_string("fn f() { let (tx, rx) = tokio::sync::mpsc::channel(1); }");
        assert!(
            out.contains("laplace_sdk")
                && out.contains("unmodeled")
                && out.contains("TOKIO_CHANNEL"),
            "tokio channel blind-spot marker missing: {out}"
        );
    }

    #[test]
    fn classify_flags_tokio_spawn_and_task_spawn_variants() {
        assert_eq!(classify("tokio::spawn"), Some(Unmodeled::TokioSpawn));
        assert_eq!(classify("tokio::task::spawn"), Some(Unmodeled::TokioSpawn));
        assert_eq!(
            classify("tokio::task::spawn_blocking"),
            Some(Unmodeled::TokioSpawn)
        );
        assert_eq!(
            classify("tokio::task::spawn_local"),
            Some(Unmodeled::TokioSpawn)
        );
        // Bare unqualified `spawn` is excluded (false-positive risk).
        assert_eq!(classify("spawn"), None);
    }

    #[test]
    fn rewrite_injects_blind_spot_marker_for_unmodeled_tokio_spawn() {
        let out = rewrite_to_string(
            "fn f() { tokio::spawn(async {}); tokio::task::spawn_blocking(|| {}); }",
        );
        assert!(
            out.contains("laplace_sdk") && out.contains("unmodeled") && out.contains("TOKIO_SPAWN"),
            "tokio spawn blind-spot marker missing: {out}"
        );
        // Deduplicated via the BTreeSet: exactly one TOKIO_SPAWN marker for
        // two distinct spawn call sites.
        assert_eq!(
            out.matches("TOKIO_SPAWN").count(),
            1,
            "expected one marker: {out}"
        );
    }

    #[test]
    fn rewrite_maps_qualified_tokio_time_functions_and_types() {
        let out = rewrite_to_string(
            "fn f() { \
                let s: tokio::time::Sleep = tokio::time::sleep(D); \
                let t: tokio::time::Timeout<X> = tokio::time::timeout(D, fut); \
                let i: tokio::time::Interval = tokio::time::interval(D); \
            }",
        );
        assert!(
            out.contains("laplace_sdk") && out.contains("rt") && out.contains("time"),
            "time module root missing: {out}"
        );
        assert!(out.contains("sleep"), "sleep fn rewrite missing: {out}");
        assert!(out.contains("timeout"), "timeout fn rewrite missing: {out}");
        assert!(
            out.contains("interval"),
            "interval fn rewrite missing: {out}"
        );
        assert!(out.contains("Sleep"), "Sleep type rewrite missing: {out}");
        assert!(
            out.contains("Timeout") && out.contains('X'),
            "Timeout<F> generic must be preserved: {out}"
        );
        assert!(
            out.contains("Interval"),
            "Interval type rewrite missing: {out}"
        );
        assert!(!out.contains("unmodeled"), "unexpected marker: {out}");
    }

    #[test]
    fn classify_does_not_flag_tokio_time_duration_or_error() {
        // Value/error types are plain uses, not un-modeled primitives.
        assert_eq!(classify("tokio::time::Duration"), None);
        assert_eq!(classify("tokio::time::error::Elapsed"), None);
        // Modeled seam functions/types must not be flagged either.
        assert_eq!(classify("tokio::time::sleep"), None);
        assert_eq!(classify("tokio::time::timeout"), None);
        assert_eq!(classify("tokio::time::interval"), None);
        assert_eq!(classify("tokio::time::Sleep"), None);
        assert_eq!(classify("tokio::time::Timeout"), None);
        assert_eq!(classify("tokio::time::Interval"), None);
    }

    #[test]
    fn classify_flags_unmodeled_tokio_time_primitives() {
        assert_eq!(classify("tokio::time::Instant"), Some(Unmodeled::TokioTime));
        assert_eq!(
            classify("tokio::time::Instant::now"),
            Some(Unmodeled::TokioTime)
        );
        assert_eq!(
            classify("tokio::time::sleep_until"),
            Some(Unmodeled::TokioTime)
        );
        assert_eq!(
            classify("tokio::time::interval_at"),
            Some(Unmodeled::TokioTime)
        );
        assert_eq!(
            classify("tokio::time::timeout_at"),
            Some(Unmodeled::TokioTime)
        );
        assert_eq!(classify("tokio::time::pause"), Some(Unmodeled::TokioTime));
        assert_eq!(classify("tokio::time::resume"), Some(Unmodeled::TokioTime));
        assert_eq!(classify("tokio::time::advance"), Some(Unmodeled::TokioTime));
    }

    #[test]
    fn rewrite_injects_blind_spot_marker_for_unmodeled_tokio_time() {
        let out = rewrite_to_string("fn f() { let _ = tokio::time::Instant::now(); }");
        assert!(
            out.contains("laplace_sdk") && out.contains("unmodeled") && out.contains("TOKIO_TIME"),
            "tokio time blind-spot marker missing: {out}"
        );
    }

    #[test]
    fn rewrite_maps_qualified_tokio_select_macro_path() {
        let out = rewrite_to_string("fn f() { tokio::select! { _ = async {} => {} } }");
        assert!(
            out.contains("laplace_sdk") && out.contains("laplace_select"),
            "select! macro path rewrite missing: {out}"
        );
        assert!(!out.contains("unmodeled"), "unexpected marker: {out}");
    }

    #[test]
    fn rewrite_does_not_touch_bare_select_macro() {
        // Unqualified bare `select!` is excluded — too high a false-positive
        // risk against an unrelated user macro of the same name (mirrors the
        // bare-`spawn` exclusion).
        let out = rewrite_to_string("fn f() { select! { _ = async {} => {} } }");
        assert!(
            !out.contains("laplace_select"),
            "bare select! must not be rewritten: {out}"
        );
    }
}
