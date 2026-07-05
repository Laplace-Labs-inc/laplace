// SPDX-License-Identifier: Apache-2.0
//! `#[axiom_harness]` — procedural macro for automatic harness registration.
//!
//! Decorating a function with this attribute leaves the original function
//! intact and appends an `inventory::submit!` block that registers a
//! `laplace_harness::registry::HarnessConfig` at link time.

use proc_macro::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream, Result};
use syn::punctuated::Punctuated;
use syn::{Expr, ItemFn, Lit, Meta, Token};

/// Parsed arguments from `#[axiom_harness(...)]`.
pub(crate) struct HarnessArgs {
    pub(crate) name: String,
    pub(crate) threads: usize,
    pub(crate) resources: usize,
    pub(crate) desc: String,
    /// Expected verdict: `"clean"` or `"bug"`.  Defaults to `"clean"`.
    pub(crate) expected: String,
    /// Optional local-only resource labels. Defaults to empty.
    pub(crate) resource_names: Vec<String>,
    /// Optional local-only thread labels. Defaults to empty.
    pub(crate) thread_names: Vec<String>,
}

fn parse_string_array(expr: &Expr) -> Vec<String> {
    let Expr::Array(array) = expr else {
        return Vec::new();
    };

    let mut values = Vec::with_capacity(array.elems.len());
    for elem in &array.elems {
        let Expr::Lit(expr_lit) = elem else {
            return Vec::new();
        };
        let Lit::Str(value) = &expr_lit.lit else {
            return Vec::new();
        };
        values.push(value.value());
    }
    values
}

impl Parse for HarnessArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut name = None;
        let mut threads = None;
        let mut resources = None;
        let mut desc = None;
        let mut expected = None;
        let mut resource_names = Vec::new();
        let mut thread_names = Vec::new();

        let metas = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;

        for meta in metas {
            if let Meta::NameValue(nv) = meta {
                let key = nv.path.get_ident().map(std::string::ToString::to_string);
                match key.as_deref() {
                    Some("name") => {
                        if let Expr::Lit(expr_lit) = &nv.value {
                            if let Lit::Str(s) = &expr_lit.lit {
                                name = Some(s.value());
                            }
                        }
                    }
                    Some("threads") => {
                        if let Expr::Lit(expr_lit) = &nv.value {
                            if let Lit::Int(i) = &expr_lit.lit {
                                threads = Some(i.base10_parse::<usize>()?);
                            }
                        }
                    }
                    Some("resources") => {
                        if let Expr::Lit(expr_lit) = &nv.value {
                            if let Lit::Int(i) = &expr_lit.lit {
                                resources = Some(i.base10_parse::<usize>()?);
                            }
                        }
                    }
                    Some("desc") => {
                        if let Expr::Lit(expr_lit) = &nv.value {
                            if let Lit::Str(s) = &expr_lit.lit {
                                desc = Some(s.value());
                            }
                        }
                    }
                    Some("expected") => {
                        if let Expr::Lit(expr_lit) = &nv.value {
                            if let Lit::Str(s) = &expr_lit.lit {
                                expected = Some(s.value());
                            }
                        }
                    }
                    Some("resource_names") => {
                        resource_names = parse_string_array(&nv.value);
                    }
                    Some("thread_names") => {
                        thread_names = parse_string_array(&nv.value);
                    }
                    _ => {}
                }
            }
        }

        Ok(HarnessArgs {
            name: name.ok_or_else(|| input.error("axiom_harness: `name` attribute is required"))?,
            threads: threads
                .ok_or_else(|| input.error("axiom_harness: `threads` attribute is required"))?,
            resources: resources
                .ok_or_else(|| input.error("axiom_harness: `resources` attribute is required"))?,
            desc: desc.unwrap_or_default(),
            expected: expected.unwrap_or_else(|| "clean".to_string()),
            resource_names,
            thread_names,
        })
    }
}

/// Register a function as a verification harness via `inventory`.
///
/// The decorated function must have the signature:
/// `fn(ThreadId, usize) -> Option<(Operation, ResourceId)>`
///
/// The macro emits the original function unchanged, followed by an
/// `inventory::submit!` block that statically registers a
/// `laplace_harness::registry::HarnessConfig`.
pub(crate) fn axiom_harness_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    use syn::parse_macro_input;

    let args = parse_macro_input!(attr as HarnessArgs);
    let func = parse_macro_input!(item as ItemFn);

    let func_ident = &func.sig.ident;
    let name = &args.name;
    let threads = args.threads;
    let resources = args.resources;
    let desc = &args.desc;
    let expected = &args.expected;
    let resource_names_lit: Vec<proc_macro2::TokenStream> =
        args.resource_names.iter().map(|s| quote! { #s }).collect();
    let thread_names_lit: Vec<proc_macro2::TokenStream> =
        args.thread_names.iter().map(|s| quote! { #s }).collect();

    let expanded = quote! {
        #func

        ::inventory::submit! {
            crate::registry::HarnessConfig {
                name: #name,
                display_name: #name,
                description: #desc,
                num_threads: #threads,
                num_resources: #resources,
                op_provider: #func_ident,
                expected: #expected,
                resource_names: &[#(#resource_names_lit),*],
                thread_names: &[#(#thread_names_lit),*],
                pc_labels: &[],
            }
        }
    };

    TokenStream::from(expanded)
}
