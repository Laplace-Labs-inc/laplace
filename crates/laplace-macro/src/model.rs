// SPDX-License-Identifier: Apache-2.0

use proc_macro::TokenStream;
use quote::quote;
use syn::parse_macro_input;
use syn::visit_mut::{self, VisitMut};
use syn::{parse_quote, Expr, ExprCall, ItemFn, Path, PathArguments, PathSegment, TypePath};

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
    ModelRewrite.visit_item_fn_mut(&mut input);
    quote!(#input).into()
}

struct ModelRewrite;

impl VisitMut for ModelRewrite {
    fn visit_expr_call_mut(&mut self, node: &mut ExprCall) {
        visit_mut::visit_expr_call_mut(self, node);

        let Expr::Path(path) = node.func.as_mut() else {
            return;
        };

        if is_supported_spawn_path(&path.path) {
            path.path = parse_quote!(::laplace_rt::spawn);
        } else if let Some(rewritten) = rewrite_mutex_constructor_path(&path.path) {
            path.path = rewritten;
        }
    }

    fn visit_type_path_mut(&mut self, node: &mut TypePath) {
        visit_mut::visit_type_path_mut(self, node);

        if let Some(rewritten) = rewrite_mutex_type_path(&node.path) {
            node.path = rewritten;
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

fn rewrite_mutex_type_path(path: &Path) -> Option<Path> {
    let segments: Vec<_> = path.segments.iter().collect();
    let [std, sync, mutex] = segments.as_slice() else {
        return None;
    };
    if std.ident != "std" || sync.ident != "sync" || mutex.ident != "Mutex" {
        return None;
    }
    if !matches!(std.arguments, PathArguments::None)
        || !matches!(sync.arguments, PathArguments::None)
    {
        return None;
    }

    Some(model_mutex_path(mutex.arguments.clone(), None))
}

fn rewrite_mutex_constructor_path(path: &Path) -> Option<Path> {
    let segments: Vec<_> = path.segments.iter().collect();
    let [std, sync, mutex, method] = segments.as_slice() else {
        return None;
    };
    if std.ident != "std" || sync.ident != "sync" || mutex.ident != "Mutex" {
        return None;
    }
    if !matches!(std.arguments, PathArguments::None)
        || !matches!(sync.arguments, PathArguments::None)
        || method.ident != "new"
        || !matches!(method.arguments, PathArguments::None)
    {
        return None;
    }

    Some(model_mutex_path(
        mutex.arguments.clone(),
        Some((*method).clone()),
    ))
}

fn model_mutex_path(arguments: PathArguments, method: Option<PathSegment>) -> Path {
    let mut path: Path = parse_quote!(::laplace_rt::ModelMutex);
    path.segments
        .last_mut()
        .expect("ModelMutex path has a segment")
        .arguments = arguments;
    if let Some(method) = method {
        path.segments.push(method);
    }
    path
}
