// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeSet;

use proc_macro::TokenStream;
use quote::quote;
use syn::parse_macro_input;
use syn::visit_mut::{self, VisitMut};
use syn::{
    parse_quote, Expr, ExprCall, Ident, ItemFn, Path, PathArguments, PathSegment, Stmt, TypePath,
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
/// Rewrites qualified `std::thread::spawn` → `::laplace_rt::spawn` and
/// `std::sync::{Mutex,RwLock}` → `::laplace_rt::{ModelMutex,ModelRwLock}`, and
/// records any recognized-but-un-modeled primitive (`Condvar`, `atomic`,
/// `mpsc`) so a compile-time blind-spot warning can be injected.
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
}

impl Unmodeled {
    /// The `::laplace_rt::unmodeled` marker constant for this primitive.
    fn marker_ident(self) -> Ident {
        let name = match self {
            Unmodeled::Condvar => "CONDVAR",
            Unmodeled::Atomic => "ATOMIC",
            Unmodeled::Channel => "CHANNEL",
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
                parse_quote!(let _ = ::laplace_rt::unmodeled::#marker;)
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
            path.path = parse_quote!(::laplace_rt::spawn);
        } else if let Some(rewritten) = rewrite_std_sync_constructor_path(&path.path) {
            path.path = rewritten;
        } else if let Some(primitive) = classify_unmodeled(&path.path) {
            self.unmodeled.insert(primitive);
        }
    }

    fn visit_type_path_mut(&mut self, node: &mut TypePath) {
        visit_mut::visit_type_path_mut(self, node);

        if let Some(rewritten) = rewrite_std_sync_type_path(&node.path) {
            node.path = rewritten;
        } else if let Some(primitive) = classify_unmodeled(&node.path) {
            self.unmodeled.insert(primitive);
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
    let mut path: Path = parse_quote!(::laplace_rt::#ident);
    path.segments
        .last_mut()
        .expect("model path has a segment")
        .arguments = arguments;
    if let Some(method) = method {
        path.segments.push(method);
    }
    path
}

/// Classifies a recognized-but-un-modeled concurrency primitive by its path
/// segments. Modeled primitives (Mutex/RwLock/spawn) return `None` and are
/// handled by the rewriters above.
fn classify_unmodeled(path: &Path) -> Option<Unmodeled> {
    let has = |name: &str| path.segments.iter().any(|segment| segment.ident == name);

    if has("Condvar") {
        return Some(Unmodeled::Condvar);
    }
    if has("mpsc") {
        return Some(Unmodeled::Channel);
    }
    let is_atomic = path.segments.iter().any(|segment| {
        let ident = segment.ident.to_string();
        ident == "atomic" || (ident.starts_with("Atomic") && ident.len() > "Atomic".len())
    });
    if is_atomic {
        return Some(Unmodeled::Atomic);
    }
    None
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
        // Modeled or unrelated paths must not be flagged.
        assert_eq!(classify("std::sync::Mutex"), None);
        assert_eq!(classify("std::sync::RwLock"), None);
        assert_eq!(classify("std::sync::Arc"), None);
        // "Atomic" alone is too generic to flag (avoids user-type false positives).
        assert_eq!(classify("Atomic"), None);
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
            out.contains("laplace_rt") && out.contains("unmodeled") && out.contains("CONDVAR"),
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
}
