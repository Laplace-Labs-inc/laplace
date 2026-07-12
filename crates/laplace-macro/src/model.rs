// SPDX-License-Identifier: Apache-2.0

use std::collections::{BTreeMap, BTreeSet};

use proc_macro::TokenStream;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::token::PathSep;
use syn::visit_mut::{self, VisitMut};
use syn::{
    parse_quote, Expr, ExprCall, Ident, Item, ItemFn, ItemMod, Macro, Path, PathArguments,
    PathSegment, Stmt, TypePath, UseTree,
};

/// `#[laplace::model]`'s dispatch over the annotated item kind: a plain `fn`
/// (the original P-1 surface) or an inline `mod { ... }` (AXM2 A2-5 — a
/// proc-macro attached to a module sees that module's own top-level `use`
/// items, so annotating the module instead of each `fn` unlocks the
/// dominant crates.io alias style — `use tokio::sync::mpsc;` +
/// `mpsc::channel(1)` — for every `fn` it contains).
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

    let parsed = match syn::parse::<Item>(item) {
        Ok(parsed) => parsed,
        Err(err) => return err.to_compile_error().into(),
    };

    match parsed {
        Item::Fn(mut func) => {
            apply_model_rewrite(&mut func);
            quote!(#func).into()
        }
        Item::Mod(mut item_mod) => match apply_model_rewrite_to_annotated_mod(&mut item_mod) {
            Ok(()) => quote!(#item_mod).into(),
            Err(err) => err.to_compile_error().into(),
        },
        other => syn::Error::new_spanned(
            &other,
            "`#[laplace::model]` only supports a `fn` or an inline `mod { ... }`; annotate a \
             function directly, or annotate an inline module so its `use` imports are visible \
             to the rewrite",
        )
        .to_compile_error()
        .into(),
    }
}

/// Applies the shared model rewrite (spawn/Mutex/RwLock routing) and injects
/// un-modeled-primitive markers, in one pass, into an annotated function.
///
/// `#[laplace::model]` and `#[laplace::verify]` share this so a single
/// attribute performs the full rewrite before any harness is emitted. No
/// enclosing `use`-import alias scope is available here (a bare `fn`
/// annotation only ever sees its own body) — see
/// [`apply_model_rewrite_with_base_aliases`] for the `mod`-annotated path
/// that threads an enclosing scope's alias table in.
pub(crate) fn apply_model_rewrite(func: &mut ItemFn) {
    apply_model_rewrite_with_options(func, ModelRewriteOptions::default());
}

/// Options for the shared model rewrite. `tasks` is intentionally opt-in:
/// only the `TaskSet` composition surface can route Tokio spawns through the
/// `TaskHandle` shadow; other verify/model modes retain their honest marker.
#[derive(Clone, Copy, Default)]
pub(crate) struct ModelRewriteOptions {
    pub(crate) tasks: bool,
}

/// Applies the shared model rewrite with an explicit mode boundary.
pub(crate) fn apply_model_rewrite_with_options(func: &mut ItemFn, options: ModelRewriteOptions) {
    apply_model_rewrite_with_base_aliases(func, &BTreeMap::new(), options);
}

/// [`apply_model_rewrite`], but merging `base_aliases` (the alias table
/// resolved from an enclosing `#[laplace::model] mod { ... }`'s own `use`
/// items, if any) with this function's own body-local `use` items before
/// the rewrite pass — a `fn`-local alias of the same binding name shadows
/// the enclosing module's.
fn apply_model_rewrite_with_base_aliases(
    func: &mut ItemFn,
    base_aliases: &BTreeMap<String, Vec<Ident>>,
    options: ModelRewriteOptions,
) {
    let aliases = merge_aliases(base_aliases, &scan_use_aliases_stmts(&func.block.stmts));
    let mut rewrite = ModelRewrite {
        aliases,
        options,
        ..ModelRewrite::default()
    };
    rewrite.visit_item_fn_mut(func);
    rewrite.inject_unmodeled_markers(func);
}

/// Applies the model rewrite to every `fn` inside an annotated inline
/// `mod { ... }`, transitively across nested inline modules, threading each
/// scope's own top-level `use` items into the alias table available to its
/// contents.
///
/// Rejects an out-of-line `mod foo;`: a proc-macro attribute only ever
/// receives the tokens of the item it is attached to, so `mod foo;`'s
/// `use` imports (and its `fn` bodies, in `foo.rs`) are not visible here —
/// silently accepting it would mean silently *not* rewriting anything in
/// that file, which is the false-green shape this crate treats as a bug.
fn apply_model_rewrite_to_annotated_mod(item_mod: &mut ItemMod) -> syn::Result<()> {
    let Some((_, items)) = &mut item_mod.content else {
        return Err(syn::Error::new_spanned(
            &item_mod.ident,
            "`#[laplace::model]` cannot see the contents of an out-of-line `mod foo;` (a \
             proc-macro only receives the annotated item's own tokens); annotate an inline \
             module (`mod foo { ... }`) or annotate the `fn` directly",
        ));
    };
    apply_model_rewrite_to_items(items, &BTreeMap::new(), ModelRewriteOptions::default());
    Ok(())
}

/// Recurses into `items` (an inline module's contents), merging
/// `base_aliases` (the enclosing scope's resolved alias table) with this
/// scope's own top-level `use` items to build the alias table available to
/// every `fn` and nested inline `mod` here. A nested scope's `use` of the
/// same binding name shadows the enclosing one's; everything else the
/// enclosing scope resolved is inherited.
///
/// A nested out-of-line `mod foo;` is left untouched — same visibility
/// limit as the top-level annotated-item case, but not rejected here (only
/// the item `#[laplace::model]` is directly attached to must be inline).
fn apply_model_rewrite_to_items(
    items: &mut [Item],
    base_aliases: &BTreeMap<String, Vec<Ident>>,
    options: ModelRewriteOptions,
) {
    let scope_aliases = merge_aliases(base_aliases, &scan_use_aliases_items(items));
    for item in items.iter_mut() {
        match item {
            Item::Fn(func) => apply_model_rewrite_with_base_aliases(func, &scope_aliases, options),
            Item::Mod(inner_mod) => {
                if let Some((_, inner_items)) = &mut inner_mod.content {
                    apply_model_rewrite_to_items(inner_items, &scope_aliases, options);
                }
            }
            _ => {}
        }
    }
}

/// `std`-qualified concurrency primitive rewriter shared by `#[laplace::model]`
/// and `#[laplace::verify]`.
///
/// Rewrites qualified `std::thread::spawn` → `::laplace_sdk::rt::spawn`,
/// `std::sync::{Mutex,RwLock}` → `::laplace_sdk::rt::{ModelMutex,ModelRwLock}`,
/// `tokio::sync::{Mutex,RwLock,Semaphore,Notify}` → their `::laplace_sdk::rt`
/// model equivalents, and `tokio::sync::{mpsc,oneshot,watch}` constructors
/// and types → their `::laplace_sdk::rt::{mpsc,oneshot,watch}` model
/// equivalents, and records any recognized-but-un-modeled primitive
/// (`Condvar`, `atomic`, `std::sync::mpsc`, `tokio::spawn`, and
/// `tokio::sync::broadcast`) so a compile-time blind-spot warning can be
/// injected.
///
/// AXM2 A2-5 adds `aliases`: a `use`-import-derived table (binding ident →
/// canonical `tokio::...` path segments, see [`canonicalize`](Self::canonicalize))
/// so the *dominant* crates.io style — `use tokio::sync::mpsc;` followed by
/// `mpsc::channel(1)` — reaches the same rewrite chain as the fully-qualified
/// form, instead of only being conservatively flagged.
#[derive(Default)]
pub(crate) struct ModelRewrite {
    unmodeled: BTreeSet<Unmodeled>,
    aliases: BTreeMap<String, Vec<Ident>>,
    options: ModelRewriteOptions,
}

impl ModelRewrite {
    /// Resolves `path`'s first segment against the `use`-import alias table
    /// for the current scope, returning the canonical `tokio::...` path with
    /// the alias segment expanded to its full canonical prefix (generic
    /// arguments on the original first segment are preserved on the
    /// corresponding trailing prefix segment). Returns `None` when the first
    /// segment is not a resolvable alias — no matching `use`, a glob import,
    /// or a scope-shadowed binding — in which case callers fall back to
    /// `path` unchanged, exactly reproducing the pre-A2-5 full-path-only
    /// behavior.
    ///
    /// Deliberately scoped to `tokio`-rooted imports only (see
    /// [`insert_tokio_alias`]) — extending this to `std::sync` would flip
    /// the `model_does_not_rewrite_bare_mutex` pass test's locked-in
    /// contract (a bare `use std::sync::Mutex; Mutex::new(..)` staying
    /// unrewritten) into a rewrite, which is out of this pass's scope.
    fn canonicalize(&self, path: &Path) -> Option<Path> {
        let first = path.segments.first()?;
        let canonical_prefix = self.aliases.get(&first.ident.to_string())?;

        let mut segments: Punctuated<PathSegment, PathSep> = Punctuated::new();
        let last_index = canonical_prefix.len().saturating_sub(1);
        for (index, ident) in canonical_prefix.iter().enumerate() {
            let arguments = if index == last_index {
                first.arguments.clone()
            } else {
                PathArguments::None
            };
            segments.push(PathSegment {
                ident: ident.clone(),
                arguments,
            });
        }
        for segment in path.segments.iter().skip(1) {
            segments.push(segment.clone());
        }

        Some(Path {
            leading_colon: None,
            segments,
        })
    }
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
    fn visit_stmt_mut(&mut self, node: &mut Stmt) {
        if self.options.tasks {
            match node {
                Stmt::Expr(expr, Some(_)) => self.rewrite_discarded_spawn(expr),
                Stmt::Local(local) if matches!(&local.pat, syn::Pat::Wild(_)) => {
                    if let Some(init) = local.init.as_mut() {
                        self.rewrite_discarded_spawn(&mut init.expr);
                    }
                }
                _ => {}
            }
        }

        visit_mut::visit_stmt_mut(self, node);
    }

    fn visit_expr_call_mut(&mut self, node: &mut ExprCall) {
        visit_mut::visit_expr_call_mut(self, node);

        let Expr::Path(path) = node.func.as_mut() else {
            return;
        };

        // AXM2 A2-5: resolve an aliased first segment (`mpsc::channel(1)`
        // after `use tokio::sync::mpsc;`) to its canonical `tokio::...`
        // shape before running the same rewrite chain the fully-qualified
        // form always used — a single target table, reused either way.
        let canonical = self.canonicalize(&path.path);
        let effective = canonical.as_ref().unwrap_or(&path.path);

        if self.options.tasks && is_tokio_spawn_path(effective) {
            path.path = parse_quote!(::laplace_sdk::rt::spawn_task);
        } else if is_supported_spawn_path(effective) {
            path.path = parse_quote!(::laplace_sdk::rt::spawn);
        } else if let Some(rewritten) = rewrite_std_sync_constructor_path(effective) {
            path.path = rewritten;
        } else if let Some(rewritten) = rewrite_tokio_sync_constructor_path(effective) {
            path.path = rewritten;
        } else if let Some(rewritten) = rewrite_tokio_sync_channel_constructor_path(effective) {
            path.path = rewritten;
        } else if let Some(rewritten) = rewrite_tokio_time_fn_path(effective) {
            path.path = rewritten;
        } else if let Some(primitive) = classify_unmodeled(effective) {
            self.unmodeled.insert(primitive);
        }
    }

    fn visit_type_path_mut(&mut self, node: &mut TypePath) {
        visit_mut::visit_type_path_mut(self, node);

        let canonical = self.canonicalize(&node.path);
        let effective = canonical.as_ref().unwrap_or(&node.path);

        if let Some(rewritten) = rewrite_std_sync_type_path(effective) {
            node.path = rewritten;
        } else if let Some(rewritten) = rewrite_tokio_sync_type_path(effective) {
            node.path = rewritten;
        } else if let Some(rewritten) = rewrite_tokio_sync_channel_type_path(effective) {
            node.path = rewritten;
        } else if let Some(rewritten) = rewrite_tokio_time_type_path(effective) {
            node.path = rewritten;
        } else if let Some(primitive) = classify_unmodeled(effective) {
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
    /// an unrelated user macro of the same name. `use tokio::select;` +
    /// bare `select! { .. }` is *not* excluded (AXM2 A2-5): the `use`
    /// import is what proves the binding actually names `tokio::select`,
    /// which is exactly the evidence the bare-macro exclusion above is
    /// missing without it — see [`ModelRewrite::canonicalize`].
    fn visit_macro_mut(&mut self, node: &mut Macro) {
        visit_mut::visit_macro_mut(self, node);

        let canonical = self.canonicalize(&node.path);
        let effective = canonical.as_ref().unwrap_or(&node.path);

        if is_tokio_select_macro_path(effective) {
            node.path = parse_quote!(::laplace_sdk::rt::laplace_select);
        }
    }
}

impl ModelRewrite {
    /// Rewrites a direct `tokio::spawn` expression. In tasks mode the returned
    /// value is the `laplace_rt::TaskHandle` shadow, so bindings, awaits, and
    /// `abort()` calls remain visible to the modelled program.
    fn rewrite_discarded_spawn(&mut self, expr: &mut Expr) {
        let Expr::Call(call) = expr else {
            return;
        };
        let Expr::Path(path) = call.func.as_mut() else {
            return;
        };

        let canonical = self.canonicalize(&path.path);
        let effective = canonical.as_ref().unwrap_or(&path.path);
        if is_tokio_spawn_path(effective) {
            path.path = parse_quote!(::laplace_sdk::rt::spawn_task);
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
/// AXM2 A2-3 slice 2; the `mpsc`/oneshot/watch channel family is modeled as
/// of AXM2 A2-4 via [`tokio_channel_fn_target_for`]/
/// [`tokio_channel_type_target_for`]. Only `tokio::sync::broadcast` remains
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

/// The `::laplace_rt` model channel module + name for a
/// `tokio::sync::{mpsc,oneshot,watch}::{channel,unbounded_channel}`
/// constructor, if supported.
fn tokio_channel_fn_target_for(
    module: &Ident,
    func: &Ident,
) -> Option<(&'static str, &'static str)> {
    match (module.to_string().as_str(), func.to_string().as_str()) {
        ("mpsc", "channel") => Some(("mpsc", "channel")),
        ("mpsc", "unbounded_channel") => Some(("mpsc", "unbounded_channel")),
        ("oneshot", "channel") => Some(("oneshot", "channel")),
        ("watch", "channel") => Some(("watch", "channel")),
        _ => None,
    }
}

/// The `::laplace_rt` model channel module + type name for a
/// `tokio::sync::{mpsc,oneshot,watch}::TYPE`, if supported. Channel-family
/// types outside this set (`error::*`, `Permit`, `OwnedPermit`,
/// `WeakSender`, ...) are intentionally left unrewritten — see the module's
/// "loud residual" honesty-contract bullets in `async_mpsc`/`async_oneshot`/
/// `async_watch`.
fn tokio_channel_type_target_for(
    module: &Ident,
    ty: &Ident,
) -> Option<(&'static str, &'static str)> {
    match (module.to_string().as_str(), ty.to_string().as_str()) {
        ("mpsc", "Sender") => Some(("mpsc", "Sender")),
        ("mpsc", "Receiver") => Some(("mpsc", "Receiver")),
        ("mpsc", "UnboundedSender") => Some(("mpsc", "UnboundedSender")),
        ("mpsc", "UnboundedReceiver") => Some(("mpsc", "UnboundedReceiver")),
        ("oneshot", "Sender") => Some(("oneshot", "Sender")),
        ("oneshot", "Receiver") => Some(("oneshot", "Receiver")),
        ("watch", "Sender") => Some(("watch", "Sender")),
        ("watch", "Receiver") => Some(("watch", "Receiver")),
        ("watch", "Ref") => Some(("watch", "Ref")),
        _ => None,
    }
}

/// Rewrites a `tokio::sync::{mpsc,oneshot,watch}::{channel,unbounded_channel}`
/// *constructor* call path to its `::laplace_rt` model equivalent,
/// preserving any turbofish generic arguments on the constructor segment.
fn rewrite_tokio_sync_channel_constructor_path(path: &Path) -> Option<Path> {
    let segments: Vec<_> = path.segments.iter().collect();
    let [tokio, sync, module, func] = segments.as_slice() else {
        return None;
    };
    if tokio.ident != "tokio" || sync.ident != "sync" {
        return None;
    }
    if !matches!(tokio.arguments, PathArguments::None)
        || !matches!(sync.arguments, PathArguments::None)
        || !matches!(module.arguments, PathArguments::None)
    {
        return None;
    }

    let (target_module, target_fn) = tokio_channel_fn_target_for(&module.ident, &func.ident)?;
    Some(channel_path(
        target_module,
        target_fn,
        func.arguments.clone(),
    ))
}

/// Rewrites a `tokio::sync::{mpsc,oneshot,watch}::TYPE` *type* path to its
/// `::laplace_rt` model equivalent, preserving generic arguments.
fn rewrite_tokio_sync_channel_type_path(path: &Path) -> Option<Path> {
    let segments: Vec<_> = path.segments.iter().collect();
    let [tokio, sync, module, ty] = segments.as_slice() else {
        return None;
    };
    if tokio.ident != "tokio" || sync.ident != "sync" {
        return None;
    }
    if !matches!(tokio.arguments, PathArguments::None)
        || !matches!(sync.arguments, PathArguments::None)
        || !matches!(module.arguments, PathArguments::None)
    {
        return None;
    }

    let (target_module, target_ty) = tokio_channel_type_target_for(&module.ident, &ty.ident)?;
    Some(channel_path(target_module, target_ty, ty.arguments.clone()))
}

/// Builds a `::laplace_sdk::rt::{module}::{name}` path, applying `arguments`
/// (generics/turbofish) to the trailing `name` segment. Mirrors
/// [`model_path`] but for the two-segment `{module}::{name}` shape used by
/// the channel family (as opposed to the lock family's single flat name).
fn channel_path(module: &str, name: &str, arguments: PathArguments) -> Path {
    let module_ident = Ident::new(module, proc_macro2::Span::call_site());
    let name_ident = Ident::new(name, proc_macro2::Span::call_site());
    let mut path: Path = parse_quote!(::laplace_sdk::rt::#module_ident::#name_ident);
    path.segments
        .last_mut()
        .expect("channel path has a segment")
        .arguments = arguments;
    path
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
/// [`rewrite_tokio_sync_type_path`] / [`rewrite_tokio_sync_constructor_path`].
/// `mpsc`/`oneshot`/`watch` are excluded here too — all three are modeled
/// and handled by [`rewrite_tokio_sync_channel_type_path`] /
/// [`rewrite_tokio_sync_channel_constructor_path`], which run before this
/// classifier in [`ModelRewrite`]'s visitor methods. Only
/// `tokio::sync::broadcast` remains un-modeled.
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

    if ty.ident == "broadcast" {
        Some(Unmodeled::TokioChannel)
    } else {
        None
    }
}

/// Whether `path` is a `tokio::spawn`/`tokio::task::spawn` call, by exact
/// plain-segment match. Unqualified bare `spawn(...)` is intentionally
/// excluded — too high a false-positive risk against an unrelated user
/// function of the same name.
fn is_tokio_spawn_path(path: &Path) -> bool {
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
                && *method == "spawn"
    )
}

/// Whether `path` is a recognized Tokio spawn variant that remains
/// unmodeled in every mode. `spawn_blocking` and `spawn_local` have distinct
/// runtime semantics and must not be routed through the fire-and-forget seam.
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
        [(tokio, _), (task, _), (method, _)]
            if *tokio == "tokio"
                && *task == "task"
                && matches!(method.to_string().as_str(), "spawn_blocking" | "spawn_local")
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
    if is_tokio_spawn_path(path) || is_unmodeled_tokio_spawn_path(path) {
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

// ── AXM2 A2-5: use-import alias tracking ────────────────────────────────────
//
// The functions below build a `binding ident -> canonical tokio::... path
// segments` table from the `use` items visible in an annotated scope (a
// `fn` body, or an inline `mod`'s own top-level items), which
// [`ModelRewrite::canonicalize`] then consults to resolve an aliased path
// to the shape the existing `rewrite_*`/`classify_unmodeled` target-table
// functions already expect — no second target table, per the single
// source of truth constraint.

/// Merges `overlay` onto `base`, with `overlay` winning on a binding-name
/// collision (an inner scope's `use` shadows an outer scope's alias of the
/// same name; everything else the outer scope resolved is inherited).
fn merge_aliases(
    base: &BTreeMap<String, Vec<Ident>>,
    overlay: &BTreeMap<String, Vec<Ident>>,
) -> BTreeMap<String, Vec<Ident>> {
    let mut merged = base.clone();
    merged.extend(overlay.iter().map(|(k, v)| (k.clone(), v.clone())));
    merged
}

/// Records `binding -> canonical` in `table`, but only when `canonical` is
/// rooted at `tokio` — deliberately excluding `std::sync` aliases (see
/// [`ModelRewrite::canonicalize`]'s doc for why).
fn insert_tokio_alias(
    table: &mut BTreeMap<String, Vec<Ident>>,
    binding: String,
    canonical: Vec<Ident>,
) {
    if canonical.first().is_some_and(|ident| ident == "tokio") {
        table.insert(binding, canonical);
    }
}

/// Recursively walks one `use` tree (a single top-level `use` item may
/// desugar to several bindings via renames/nested groups), recording every
/// leaf binding's canonical path into `table`. `prefix` accumulates the
/// path segments seen on the way down; glob imports (`use tokio::sync::*;`)
/// are intentionally not recorded — an unresolvable wildcard, so the
/// existing conservative marker path stays in effect for names it would
/// have covered.
fn collect_use_aliases(
    tree: &UseTree,
    prefix: &mut Vec<Ident>,
    table: &mut BTreeMap<String, Vec<Ident>>,
) {
    match tree {
        UseTree::Path(use_path) => {
            prefix.push(use_path.ident.clone());
            collect_use_aliases(&use_path.tree, prefix, table);
            prefix.pop();
        }
        UseTree::Name(use_name) => {
            if use_name.ident == "self" {
                // `use tokio::sync::{self, mpsc};` binds `sync` itself to
                // `tokio::sync` — the leaf name is the last prefix segment,
                // not literally `self`.
                if let Some(binding) = prefix.last().cloned() {
                    insert_tokio_alias(table, binding.to_string(), prefix.clone());
                }
                return;
            }
            let mut canonical = prefix.clone();
            canonical.push(use_name.ident.clone());
            insert_tokio_alias(table, use_name.ident.to_string(), canonical);
        }
        UseTree::Rename(use_rename) => {
            let mut canonical = prefix.clone();
            canonical.push(use_rename.ident.clone());
            insert_tokio_alias(table, use_rename.rename.to_string(), canonical);
        }
        UseTree::Group(use_group) => {
            for item in &use_group.items {
                collect_use_aliases(item, prefix, table);
            }
        }
        UseTree::Glob(_) => {}
    }
}

/// The declared ident of an item that can shadow a `use` binding of the
/// same name (a `let` binding is deliberately not in scope here — only
/// item-level declarations count, per AXM2 A2-5's shadowing contract).
/// Item kinds without a single top-level ident (`impl`, `use` itself,
/// macros, ...) are not covered; a same-name collision through one of
/// those is a documented scan limitation, not a soundness bug (worst case:
/// a still-correct-but-unrewritten conservative marker).
fn item_ident(item: &Item) -> Option<&Ident> {
    match item {
        Item::Fn(i) => Some(&i.sig.ident),
        Item::Struct(i) => Some(&i.ident),
        Item::Enum(i) => Some(&i.ident),
        Item::Union(i) => Some(&i.ident),
        Item::Mod(i) => Some(&i.ident),
        Item::Type(i) => Some(&i.ident),
        Item::Const(i) => Some(&i.ident),
        Item::Static(i) => Some(&i.ident),
        Item::Trait(i) => Some(&i.ident),
        Item::TraitAlias(i) => Some(&i.ident),
        _ => None,
    }
}

/// Removes any alias whose binding name collides with an item declared in
/// the same scope — the local declaration shadows the `use` import within
/// that scope, so rewriting through the alias would be unsound there.
fn remove_shadowed_aliases<'a>(
    table: &mut BTreeMap<String, Vec<Ident>>,
    item_idents: impl Iterator<Item = &'a Ident>,
) {
    for ident in item_idents {
        table.remove(&ident.to_string());
    }
}

/// Builds the alias table visible directly inside a `fn` body: every
/// top-level `use` statement in `stmts`, minus any binding shadowed by a
/// sibling item declaration in the same body.
fn scan_use_aliases_stmts(stmts: &[Stmt]) -> BTreeMap<String, Vec<Ident>> {
    let mut table = BTreeMap::new();
    for stmt in stmts {
        if let Stmt::Item(Item::Use(item_use)) = stmt {
            collect_use_aliases(&item_use.tree, &mut Vec::new(), &mut table);
        }
    }
    let item_idents = stmts.iter().filter_map(|stmt| match stmt {
        Stmt::Item(item) => item_ident(item),
        _ => None,
    });
    remove_shadowed_aliases(&mut table, item_idents);
    table
}

/// Builds the alias table visible directly inside an inline `mod`'s own
/// items: every top-level `use` item in `items`, minus any binding
/// shadowed by a sibling item declaration in the same module.
fn scan_use_aliases_items(items: &[Item]) -> BTreeMap<String, Vec<Ident>> {
    let mut table = BTreeMap::new();
    for item in items {
        if let Item::Use(item_use) = item {
            collect_use_aliases(&item_use.tree, &mut Vec::new(), &mut table);
        }
    }
    remove_shadowed_aliases(&mut table, items.iter().filter_map(item_ident));
    table
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

    fn rewrite_to_string_with_tasks(func: &str, tasks: bool) -> String {
        let mut func: ItemFn = syn::parse_str(func).expect("valid fn");
        apply_model_rewrite_with_options(&mut func, ModelRewriteOptions { tasks });
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
    fn rewrite_maps_qualified_tokio_channel_constructors() {
        let out = rewrite_to_string(
            "fn f() { \
                let (tx1, rx1) = tokio::sync::mpsc::channel::<u8>(1); \
                let (tx2, rx2) = tokio::sync::mpsc::unbounded_channel::<u8>(); \
                let (tx3, rx3) = tokio::sync::oneshot::channel::<u8>(); \
                let (tx4, rx4) = tokio::sync::watch::channel(0u8); \
            }",
        );
        assert!(
            out.contains("laplace_sdk :: rt :: mpsc :: channel"),
            "mpsc::channel rewrite missing: {out}"
        );
        assert!(
            out.contains("laplace_sdk :: rt :: mpsc :: unbounded_channel"),
            "mpsc::unbounded_channel rewrite missing: {out}"
        );
        assert!(
            out.contains("laplace_sdk :: rt :: oneshot :: channel"),
            "oneshot::channel rewrite missing: {out}"
        );
        assert!(
            out.contains("laplace_sdk :: rt :: watch :: channel"),
            "watch::channel rewrite missing: {out}"
        );
        // Turbofish generics on the constructor segment must survive the rewrite.
        assert!(
            out.contains("channel :: < u8 >"),
            "turbofish generics lost on constructor rewrite: {out}"
        );
        assert!(!out.contains("unmodeled"), "unexpected marker: {out}");
    }

    #[test]
    fn rewrite_maps_qualified_tokio_channel_types() {
        let out = rewrite_to_string(
            "fn f( \
                a: tokio::sync::mpsc::Sender<u8>, \
                b: tokio::sync::mpsc::Receiver<u8>, \
                c: tokio::sync::mpsc::UnboundedSender<u8>, \
                d: tokio::sync::mpsc::UnboundedReceiver<u8>, \
                e: tokio::sync::oneshot::Sender<u8>, \
                f: tokio::sync::oneshot::Receiver<u8>, \
                g: tokio::sync::watch::Sender<u8>, \
                h: tokio::sync::watch::Receiver<u8>, \
            ) { }",
        );
        for expected in [
            "laplace_sdk :: rt :: mpsc :: Sender",
            "laplace_sdk :: rt :: mpsc :: Receiver",
            "laplace_sdk :: rt :: mpsc :: UnboundedSender",
            "laplace_sdk :: rt :: mpsc :: UnboundedReceiver",
            "laplace_sdk :: rt :: oneshot :: Sender",
            "laplace_sdk :: rt :: oneshot :: Receiver",
            "laplace_sdk :: rt :: watch :: Sender",
            "laplace_sdk :: rt :: watch :: Receiver",
        ] {
            assert!(
                out.contains(expected),
                "{expected} type rewrite missing: {out}"
            );
        }
        assert!(!out.contains("unmodeled"), "unexpected marker: {out}");
    }

    #[test]
    fn rewrite_injects_blind_spot_marker_for_unmodeled_tokio_broadcast_channel() {
        let out =
            rewrite_to_string("fn f() { let (tx, rx) = tokio::sync::broadcast::channel(1); }");
        assert!(
            out.contains("laplace_sdk")
                && out.contains("unmodeled")
                && out.contains("TOKIO_CHANNEL"),
            "tokio broadcast blind-spot marker missing: {out}"
        );
        // The modeled mpsc/oneshot/watch family must not also be rewritten
        // here (broadcast is the only channel primitive left unmodeled).
        assert!(
            !out.contains("laplace_sdk :: rt :: mpsc")
                && !out.contains("laplace_sdk :: rt :: oneshot")
                && !out.contains("laplace_sdk :: rt :: watch"),
            "unrelated channel modules must not be rewritten: {out}"
        );
    }

    #[test]
    fn rewrite_maps_aliased_channel_call_via_use_import() {
        // AXM2 A2-5 premise change from the old
        // `rewrite_does_not_rewrite_aliased_or_unqualified_channel_calls`
        // test this replaces: crates.io's dominant real-world style is
        // `use tokio::sync::mpsc;` + `mpsc::channel(1)` (34 vs. 3 for the
        // fully-qualified form), so this is now the *rewrite* case — the
        // `use` import is the evidence that resolves the alias to a
        // canonical `tokio::sync::mpsc::channel` path before the same
        // target-table chain the fully-qualified form always used.
        let out = rewrite_to_string(
            "fn f() { \
                use tokio::sync::mpsc; \
                let (tx, rx) = mpsc::channel::<u8>(1); \
            }",
        );
        assert!(
            out.contains("laplace_sdk :: rt :: mpsc :: channel"),
            "aliased mpsc::channel call must be rewritten: {out}"
        );
        // Turbofish generics on the call segment must survive, same as the
        // fully-qualified constructor rewrite.
        assert!(
            out.contains("channel :: < u8 >"),
            "turbofish generics lost on aliased rewrite: {out}"
        );
        assert!(!out.contains("unmodeled"), "unexpected marker: {out}");
    }

    #[test]
    fn rewrite_does_not_rewrite_unqualified_channel_calls_without_use_import() {
        // Without a `use` import as evidence, a bare `mpsc::channel(1)` call
        // is genuinely ambiguous between `std::sync::mpsc` and
        // `tokio::sync::mpsc` — the pre-existing conservative behavior for
        // this case is preserved exactly: no rewrite, honest over-flagging
        // via the generic `mpsc`-segment heuristic.
        let out = rewrite_to_string("fn f() { let (tx, rx) = mpsc::channel::<u8>(1); }");
        assert!(
            !out.contains("laplace_sdk :: rt :: mpsc"),
            "unqualified channel call must not be rewritten without a use import: {out}"
        );
        assert!(
            out.contains("unmodeled") && out.contains("CHANNEL"),
            "unqualified channel call must still be conservatively flagged: {out}"
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
    fn tasks_rewrite_discarded_tokio_spawn_calls_to_spawn_task() {
        let out = rewrite_to_string_with_tasks(
            "fn f() { tokio::spawn(async {}); let _ = tokio::task::spawn(async {}); }",
            true,
        );
        assert_eq!(
            out.matches("laplace_sdk :: rt :: spawn_task").count(),
            2,
            "discarded tasks-mode spawns must use spawn_task: {out}"
        );
        assert!(
            !out.contains("TOKIO_SPAWN"),
            "unexpected spawn marker: {out}"
        );
    }

    #[test]
    fn tasks_rewrite_join_handle_tokio_spawn_to_task_handle_shadow() {
        let out = rewrite_to_string_with_tasks(
            "fn f() { let handle = tokio::spawn(async {}); let _ = tokio::spawn(async {}).await; handle.abort(); }",
            true,
        );
        assert!(
            out.matches("laplace_sdk :: rt :: spawn_task").count() == 2,
            "JoinHandle-using spawns must use the TaskHandle shadow: {out}"
        );
        assert!(
            !out.contains("TOKIO_SPAWN"),
            "JoinHandle-using spawns must not retain the blind-spot marker: {out}"
        );
    }

    #[test]
    fn non_tasks_modes_keep_tokio_spawn_marker() {
        let out = rewrite_to_string_with_tasks("fn f() { tokio::spawn(async {}); }", false);
        assert!(
            out.contains("TOKIO_SPAWN"),
            "non-tasks modes must retain the tokio spawn marker: {out}"
        );
        assert!(
            !out.contains("laplace_sdk :: rt :: spawn_task"),
            "non-tasks modes must not rewrite tokio spawn: {out}"
        );
    }

    #[test]
    fn tasks_keep_spawn_blocking_and_spawn_local_markers() {
        let out = rewrite_to_string_with_tasks(
            "fn f() { tokio::task::spawn_blocking(|| {}); let _ = tokio::task::spawn_local(async {}); }",
            true,
        );
        assert!(
            out.contains("TOKIO_SPAWN"),
            "blocking/local spawns must retain the blind-spot marker: {out}"
        );
        assert!(
            !out.contains("laplace_sdk :: rt :: spawn_task"),
            "blocking/local spawns must not use the fire-and-forget seam: {out}"
        );
    }

    #[test]
    fn tasks_rewrite_reaches_spawns_inside_nested_async_blocks() {
        let out = rewrite_to_string_with_tasks(
            "fn f(tasks: &mut TaskSet) { tasks.spawn(async { tokio::spawn(async {}); }); }",
            true,
        );
        assert!(
            out.contains("laplace_sdk :: rt :: spawn_task"),
            "the composition-body nesting (`tasks.spawn(async {{ .. }})`) must reach the rewrite: {out}"
        );
        assert!(
            !out.contains("TOKIO_SPAWN"),
            "unexpected spawn marker: {out}"
        );
    }

    #[test]
    fn tasks_rewrite_tokio_task_spawn_alias_in_discarded_statement() {
        let out = rewrite_to_string_with_tasks(
            "fn f() { use tokio::task; let _ = task::spawn(async {}); }",
            true,
        );
        assert!(
            out.contains("laplace_sdk :: rt :: spawn_task"),
            "tokio::task alias must reach the tasks-only rewrite: {out}"
        );
        assert!(
            !out.contains("TOKIO_SPAWN"),
            "unexpected spawn marker: {out}"
        );
    }

    #[test]
    fn tasks_rewrite_tokio_task_spawn_alias_when_joined_and_aborted() {
        let out = rewrite_to_string_with_tasks(
            "fn f() { use tokio::task; let handle = task::spawn(async {}); handle.abort(); let _ = handle.await.unwrap(); }",
            true,
        );
        assert!(
            out.contains("laplace_sdk :: rt :: spawn_task"),
            "tokio::task alias must preserve the TaskHandle shadow: {out}"
        );
        assert!(
            out.contains("abort") && out.contains("await") && out.contains("unwrap"),
            "handle operations must remain visible after rewrite: {out}"
        );
        assert!(
            !out.contains("TOKIO_SPAWN"),
            "unexpected spawn marker: {out}"
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

    // ── AXM2 A2-5: use-import alias resolution ──────────────────────────

    /// Applies [`apply_model_rewrite_to_annotated_mod`] to a parsed inline
    /// `mod { ... }` and returns the emitted source as a
    /// whitespace-normalized string, mirroring [`rewrite_to_string`] for
    /// the `fn`-annotated path.
    fn rewrite_mod_to_string(module: &str) -> String {
        let mut item_mod: ItemMod = syn::parse_str(module).expect("valid inline mod");
        apply_model_rewrite_to_annotated_mod(&mut item_mod).expect("inline mod rewrite succeeds");
        item_mod.into_token_stream().to_string()
    }

    #[test]
    fn rewrite_maps_aliased_oneshot_channel_via_use_import() {
        let out = rewrite_to_string(
            "fn f() { \
                use tokio::sync::oneshot; \
                let (tx, rx) = oneshot::channel::<u8>(); \
            }",
        );
        assert!(
            out.contains("laplace_sdk :: rt :: oneshot :: channel"),
            "aliased oneshot::channel call must be rewritten: {out}"
        );
        assert!(!out.contains("unmodeled"), "unexpected marker: {out}");
    }

    #[test]
    fn rewrite_maps_aliased_watch_channel_via_use_import() {
        let out = rewrite_to_string(
            "fn f() { \
                use tokio::sync::watch; \
                let (tx, rx) = watch::channel(0u8); \
            }",
        );
        assert!(
            out.contains("laplace_sdk :: rt :: watch :: channel"),
            "aliased watch::channel call must be rewritten: {out}"
        );
        assert!(!out.contains("unmodeled"), "unexpected marker: {out}");
    }

    #[test]
    fn rewrite_maps_aliased_time_sleep_via_use_import() {
        let out = rewrite_to_string(
            "fn f() { \
                use tokio::time; \
                let s: tokio::time::Sleep = time::sleep(D); \
            }",
        );
        assert!(
            out.contains("laplace_sdk :: rt :: time :: sleep"),
            "aliased time::sleep call must be rewritten: {out}"
        );
        assert!(!out.contains("unmodeled"), "unexpected marker: {out}");
    }

    #[test]
    fn rewrite_maps_aliased_mutex_via_renamed_use_import() {
        // `use tokio::sync::Mutex as TMutex;` — the rename binding `TMutex`
        // must resolve to the same canonical `tokio::sync::Mutex` target as
        // the unrenamed alias / fully-qualified forms.
        let out = rewrite_to_string(
            "fn f() { \
                use tokio::sync::Mutex as TMutex; \
                let m: TMutex<u8> = TMutex::new(0); \
            }",
        );
        assert!(
            out.contains("ModelAsyncMutex"),
            "renamed tokio mutex alias must be rewritten: {out}"
        );
        assert!(!out.contains("unmodeled"), "unexpected marker: {out}");
    }

    #[test]
    fn rewrite_maps_single_segment_import_of_channel_constructor() {
        // `use tokio::sync::mpsc::channel;` binds the constructor fn
        // itself, so the call site is a bare 1-segment `channel(1)`.
        let out = rewrite_to_string(
            "fn f() { \
                use tokio::sync::mpsc::channel; \
                let (tx, rx) = channel::<u8>(1); \
            }",
        );
        assert!(
            out.contains("laplace_sdk :: rt :: mpsc :: channel"),
            "1-segment channel import call must be rewritten: {out}"
        );
        assert!(
            out.contains("channel :: < u8 >"),
            "turbofish generics lost on 1-segment alias rewrite: {out}"
        );
        assert!(!out.contains("unmodeled"), "unexpected marker: {out}");
    }

    #[test]
    fn rewrite_maps_bare_select_after_use_tokio_select_import() {
        // `use tokio::select;` is the evidence that a subsequent bare
        // `select! { .. }` actually names `tokio::select!`, not an
        // unrelated user macro of the same name — the exclusion in
        // `rewrite_does_not_touch_bare_select_macro` only applies without
        // that evidence.
        let out = rewrite_to_string(
            "fn f() { \
                use tokio::select; \
                select! { _ = async {} => {} } \
            }",
        );
        assert!(
            out.contains("laplace_sdk") && out.contains("laplace_select"),
            "select! after `use tokio::select;` must be rewritten: {out}"
        );
        assert!(!out.contains("unmodeled"), "unexpected marker: {out}");
    }

    #[test]
    fn rewrite_leaves_glob_import_as_conservative_marker() {
        // `use tokio::sync::*;` cannot be resolved to a single canonical
        // binding — no alias table entry is recorded, so the pre-existing
        // conservative marker path stays in effect exactly as it did
        // before A2-5.
        let out = rewrite_to_string(
            "fn f() { \
                use tokio::sync::*; \
                let (tx, rx) = mpsc::channel::<u8>(1); \
            }",
        );
        assert!(
            !out.contains("laplace_sdk :: rt :: mpsc"),
            "glob-imported channel call must not be rewritten: {out}"
        );
        assert!(
            out.contains("unmodeled") && out.contains("CHANNEL"),
            "glob-imported channel call must still be conservatively flagged: {out}"
        );
    }

    #[test]
    fn rewrite_maps_aliased_broadcast_channel_to_unmodeled_marker() {
        // A resolved alias that lands on a still-un-modeled tokio primitive
        // (`broadcast`, unlike `mpsc`/`oneshot`/`watch`) must classify to
        // the same `TOKIO_CHANNEL` marker the fully-qualified form gets —
        // canonicalization must feed `classify_unmodeled` too, not only the
        // `rewrite_*` functions.
        let out = rewrite_to_string(
            "fn f() { \
                use tokio::sync::broadcast; \
                let (tx, rx) = broadcast::channel::<u8>(1); \
            }",
        );
        assert!(
            !out.contains("laplace_sdk :: rt :: broadcast"),
            "broadcast must not be rewritten as if modeled: {out}"
        );
        assert!(
            out.contains("unmodeled") && out.contains("TOKIO_CHANNEL"),
            "aliased broadcast call must be flagged TOKIO_CHANNEL: {out}"
        );
    }

    #[test]
    fn rewrite_skips_alias_shadowed_by_local_item_declaration() {
        // A local item declaration with the same name as the `use` binding
        // shadows the import within this scope — the conservative choice is
        // to skip the alias entirely rather than risk an incorrect rewrite.
        let out = rewrite_to_string(
            "fn f() { \
                use tokio::sync::Mutex; \
                struct Mutex; \
                let m = Mutex::new(0); \
            }",
        );
        assert!(
            !out.contains("ModelAsyncMutex") && !out.contains("laplace_sdk :: rt :: Mutex"),
            "shadowed Mutex alias must not be rewritten: {out}"
        );
    }

    #[test]
    fn rewrite_mod_annotation_applies_module_use_to_every_contained_fn() {
        let out = rewrite_mod_to_string(
            "mod target { \
                use tokio::sync::mpsc; \
                fn f1() { let (tx, rx) = mpsc::channel::<u8>(1); } \
                fn f2() { let (tx, rx) = mpsc::channel::<u8>(2); } \
            }",
        );
        assert_eq!(
            out.matches("laplace_sdk :: rt :: mpsc :: channel").count(),
            2,
            "both fns inside the annotated mod must be rewritten via the module-level use: {out}"
        );
        assert!(!out.contains("unmodeled"), "unexpected marker: {out}");
    }

    #[test]
    fn rewrite_mod_annotation_rejects_out_of_line_mod() {
        let mut item_mod: ItemMod = syn::parse_str("mod target;").expect("valid mod decl");
        let err = apply_model_rewrite_to_annotated_mod(&mut item_mod)
            .expect_err("out-of-line mod must be rejected");
        assert!(
            err.to_string().contains("out-of-line"),
            "error must name the out-of-line mod limitation: {err}"
        );
    }

    #[test]
    fn rewrite_nested_inline_mod_scope_accumulates_outer_use_import() {
        // The inner module's own `use tokio::sync::oneshot;` must combine
        // with (not replace) the outer module's `use tokio::sync::mpsc;` —
        // AXM2 A2-5's "inner use accumulates over outer" nested-scope
        // contract.
        let out = rewrite_mod_to_string(
            "mod outer { \
                use tokio::sync::mpsc; \
                mod inner { \
                    use tokio::sync::oneshot; \
                    fn f() { \
                        let (tx1, rx1) = mpsc::channel::<u8>(1); \
                        let (tx2, rx2) = oneshot::channel::<u8>(); \
                    } \
                } \
            }",
        );
        assert!(
            out.contains("laplace_sdk :: rt :: mpsc :: channel"),
            "inner fn must see the outer module's mpsc alias: {out}"
        );
        assert!(
            out.contains("laplace_sdk :: rt :: oneshot :: channel"),
            "inner fn must see its own module's oneshot alias: {out}"
        );
        assert!(!out.contains("unmodeled"), "unexpected marker: {out}");
    }
}
