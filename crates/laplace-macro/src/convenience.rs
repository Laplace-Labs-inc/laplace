// SPDX-License-Identifier: Apache-2.0
//! Function-like convenience macros for BYOC tracked primitives.

use proc_macro::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, LitStr, Result, Token};

struct PrimitiveInput {
    value: Expr,
    name: Option<LitStr>,
}

impl Parse for PrimitiveInput {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let value = input.parse()?;
        let name = if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            Some(input.parse()?)
        } else {
            None
        };
        Ok(Self { value, name })
    }
}

pub fn mutex_impl(input: TokenStream) -> TokenStream {
    let PrimitiveInput { value, name } = syn::parse_macro_input!(input as PrimitiveInput);
    let name = name.unwrap_or_else(|| LitStr::new("laplace_mutex", proc_macro2::Span::call_site()));

    quote! {
        ::std::sync::Arc::new(::laplace_probe_sdk::__macro_support::TrackedMutex::named(#value, #name))
    }
    .into()
}

pub fn rwlock_impl(input: TokenStream) -> TokenStream {
    let PrimitiveInput { value, name } = syn::parse_macro_input!(input as PrimitiveInput);
    let name =
        name.unwrap_or_else(|| LitStr::new("laplace_rwlock", proc_macro2::Span::call_site()));

    quote! {
        ::std::sync::Arc::new(::laplace_probe_sdk::__macro_support::TrackedRwLock::named(#value, #name))
    }
    .into()
}
