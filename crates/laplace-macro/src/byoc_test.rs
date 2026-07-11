// SPDX-License-Identifier: Apache-2.0
//! `#[laplace_byoc_test]` — BYOC test boilerplate eliminator.
//!
//! Generates a single `#[test]` wrapper that:
//! 1) creates a probe event channel,
//! 2) injects `byoc_thread!` macro for per-thread setup,
//! 3) executes original function body,
//! 4) runs DPOR verification with configured expectation.

use proc_macro::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream, Result};
use syn::spanned::Spanned;
use syn::{Expr, Ident, ItemFn, Lit, ReturnType, Token};

/// Parsed arguments from `#[laplace_byoc_test(...)]`.
pub(crate) struct ByocTestArgs {
    pub(crate) name: Option<String>,
    pub(crate) expected: String,
    pub(crate) write_ard: bool,
    pub(crate) output_dir: Option<String>,
    pub(crate) buffer: usize,
    pub(crate) max_depth: Option<usize>,
    pub(crate) lock_events_only: bool,
}

impl Parse for ByocTestArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut name = None;
        let mut expected = "clean".to_string();
        let mut write_ard = false;
        let mut output_dir = None;
        let mut buffer = 8192usize;
        let mut max_depth = None;
        let mut lock_events_only = false;

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            let key_name = key.to_string();
            let _: Token![=] = input.parse()?;
            let value: Expr = input.parse()?;

            match key_name.as_str() {
                "name" => {
                    let v = parse_lit_str(&value, "name")?;
                    if name.replace(v).is_some() {
                        return Err(syn::Error::new(key.span(), "duplicate `name` argument"));
                    }
                }
                "expected" => {
                    let v = parse_lit_str(&value, "expected")?;
                    if v != "clean" && v != "bug" {
                        return Err(syn::Error::new(
                            value.span(),
                            "expected must be \"clean\" or \"bug\"",
                        ));
                    }
                    expected = v;
                }
                "write_ard" => {
                    write_ard = parse_lit_bool(&value, "write_ard")?;
                }
                "output_dir" => {
                    output_dir = Some(parse_lit_str(&value, "output_dir")?);
                }
                "buffer" => {
                    buffer = parse_lit_usize(&value, "buffer")?;
                }
                "max_depth" => {
                    max_depth = Some(parse_lit_usize(&value, "max_depth")?);
                }
                "lock_events_only" => {
                    lock_events_only = parse_lit_bool(&value, "lock_events_only")?;
                }
                _ => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown argument `{key_name}` (allowed: name, expected, write_ard, output_dir, buffer, max_depth, lock_events_only)"
                        ),
                    ));
                }
            }

            if input.peek(Token![,]) {
                let _: Token![,] = input.parse()?;
            }
        }

        Ok(Self {
            name,
            expected,
            write_ard,
            output_dir,
            buffer,
            max_depth,
            lock_events_only,
        })
    }
}

fn parse_lit_str(expr: &Expr, field: &str) -> Result<String> {
    if let Expr::Lit(expr_lit) = expr {
        if let Lit::Str(v) = &expr_lit.lit {
            return Ok(v.value());
        }
    }
    Err(syn::Error::new(
        expr.span(),
        format!("`{field}` must be a string literal"),
    ))
}

fn parse_lit_bool(expr: &Expr, field: &str) -> Result<bool> {
    if let Expr::Lit(expr_lit) = expr {
        if let Lit::Bool(v) = &expr_lit.lit {
            return Ok(v.value());
        }
    }
    Err(syn::Error::new(
        expr.span(),
        format!("`{field}` must be a bool literal"),
    ))
}

fn parse_lit_usize(expr: &Expr, field: &str) -> Result<usize> {
    if let Expr::Lit(expr_lit) = expr {
        if let Lit::Int(v) = &expr_lit.lit {
            return v.base10_parse::<usize>();
        }
    }
    Err(syn::Error::new(
        expr.span(),
        format!("`{field}` must be an integer literal"),
    ))
}

pub(crate) fn laplace_byoc_test_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    use syn::parse_macro_input;

    let args = parse_macro_input!(attr as ByocTestArgs);
    let func = parse_macro_input!(item as ItemFn);

    if func.sig.asyncness.is_some() {
        return syn::Error::new(
            func.sig.fn_token.span(),
            "laplace_byoc_test: async fn is not supported; use a sync test function",
        )
        .to_compile_error()
        .into();
    }
    if !func.sig.inputs.is_empty() {
        return syn::Error::new(
            func.sig.ident.span(),
            "laplace_byoc_test: function must not take parameters",
        )
        .to_compile_error()
        .into();
    }
    if !matches!(func.sig.output, ReturnType::Default) {
        return syn::Error::new(
            func.sig.ident.span(),
            "laplace_byoc_test: function must return `()`",
        )
        .to_compile_error()
        .into();
    }

    let func_ident = &func.sig.ident;
    let func_body = &func.block;
    let target_name = args.name.unwrap_or_else(|| func_ident.to_string());
    let write_ard = args.write_ard;
    let buffer = args.buffer;
    let lock_events_only = args.lock_events_only;

    let output_dir_expr = if let Some(output_dir) = args.output_dir {
        quote! { #output_dir.to_string() }
    } else {
        quote! { ::std::env::temp_dir().to_string_lossy().into_owned() }
    };

    let max_depth_field = if let Some(max_depth) = args.max_depth {
        quote! { max_depth: #max_depth, }
    } else {
        quote! {}
    };

    let expected = args.expected;

    let expanded = quote! {
        #[cfg(test)]
        #[test]
        #[allow(non_snake_case)]
        fn #func_ident() {
            use ::std::sync::mpsc;
            use ::laplace_sdk::__macro_support::{
                ProbeEvent,
                ProbeSessionConfig,
                set_probe_sender,
                set_probe_thread_id,
            };

            let (__byoc_tx, __byoc_rx) = mpsc::sync_channel::<ProbeEvent>(#buffer);

            #[allow(unused_macros)]
            macro_rules! byoc_thread {
                ($id:expr, $body:block) => {{
                    let __tx = __byoc_tx.clone();
                    ::std::thread::spawn(move || {
                        set_probe_sender(__tx);
                        set_probe_thread_id($id as u64);
                        $body
                    })
                }};
            }

            #func_body

            drop(__byoc_tx);
            let __byoc_events: Vec<ProbeEvent> = __byoc_rx.into_iter().collect();
            let __byoc_events: Vec<ProbeEvent> = if #lock_events_only {
                __byoc_events
                    .into_iter()
                    .filter(|e| {
                        matches!(
                            e,
                            ProbeEvent::LockAcquired { .. } | ProbeEvent::LockReleased { .. }
                        )
                    })
                    .collect()
            } else {
                __byoc_events
            };

            if __byoc_events.is_empty() {
                eprintln!(
                    "[laplace_byoc_test] WARNING: 0 events for '{}'. Check that TrackedMutex/RwLock are being used.",
                    #target_name
                );
            }

            let __byoc_config = ProbeSessionConfig {
                write_ard: #write_ard,
                output_dir: #output_dir_expr,
                #max_depth_field
                ..ProbeSessionConfig::default()
            };
            let _ = (#expected, #target_name, __byoc_config, __byoc_events);
        }
    };

    TokenStream::from(expanded)
}
