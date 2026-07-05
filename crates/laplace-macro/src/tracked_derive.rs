// SPDX-License-Identifier: Apache-2.0
//! `#[laplace_tracked]` — attribute macro for automatic Tracked* type substitution.
//!
//! Transforms fields with `#[track]` attributes from standard sync primitives
//! (`Mutex`, `RwLock`, `Atomic*`, `Semaphore`) to their `Tracked*` equivalents, and
//! generates a `Default` impl that instantiates each tracked field with
//! appropriate resource names.

use quote::quote;
use std::collections::HashSet;
use syn::punctuated::Punctuated;
use syn::{
    Attribute, Error, Expr, Fields, GenericArgument, ItemStruct, Lit, Meta, PathArguments, Result,
    Token, Type,
};

#[derive(Debug, Default)]
struct TrackOptions {
    name: Option<String>,
    permits: Option<usize>,
}

/// Expand `#[laplace_tracked]` attribute macro.
pub(crate) fn expand_attribute(
    _attr: proc_macro2::TokenStream,
    item: ItemStruct,
) -> Result<proc_macro2::TokenStream> {
    let struct_name = item.ident.clone();
    let mut modified_fields = Vec::new();
    let mut default_assignments = Vec::new();
    let mut resource_names = HashSet::new();

    // Process all fields
    if let Fields::Named(ref fields_named) = item.fields {
        for field in &fields_named.named {
            let field_name = field.ident.as_ref().unwrap();
            let field_name_str = field_name.to_string();

            // Check if field has #[track] attribute
            let has_track = field.attrs.iter().any(|attr| attr.path().is_ident("track"));

            if has_track {
                // Parse #[track] for optional name override and primitive-specific knobs.
                let track_options = extract_track_options(&field.attrs)?;
                let resource_name = track_options
                    .name
                    .clone()
                    .unwrap_or_else(|| field_name_str.clone());

                // Check for duplicate resource names
                if !resource_names.insert(resource_name.clone()) {
                    return Err(Error::new_spanned(
                        field,
                        format!("duplicate resource name: '{resource_name}'"),
                    ));
                }

                // Map field type to Tracked* type
                let (tracked_type, default_code) =
                    map_field_type_to_tracked(&field.ty, &resource_name, &track_options)?;

                // Create modified field (without #[track] attribute)
                let mut modified_field = field.clone();
                modified_field.ty = tracked_type;
                modified_field
                    .attrs
                    .retain(|attr| !attr.path().is_ident("track"));
                modified_fields.push(modified_field);

                // Add to Default impl assignments
                default_assignments.push(quote! {
                    #field_name: #default_code
                });
            } else {
                // Field without #[track] — use T::default()
                modified_fields.push(field.clone());
                default_assignments.push(quote! {
                    #field_name: ::std::default::Default::default()
                });
            }
        }
    }

    // Reconstruct the struct with modified fields
    let modified_item = ItemStruct {
        fields: Fields::Named(syn::FieldsNamed {
            brace_token: match &item.fields {
                Fields::Named(fn_) => fn_.brace_token,
                _ => unreachable!(),
            },
            named: modified_fields.into_iter().collect(),
        }),
        ..item
    };

    // Generate Default impl
    let (impl_generics, ty_generics, where_clause) = modified_item.generics.split_for_impl();
    let default_impl = quote! {
        impl #impl_generics ::std::default::Default for #struct_name #ty_generics #where_clause {
            fn default() -> Self {
                Self {
                    #(#default_assignments),*
                }
            }
        }
    };

    let expanded = quote! {
        #modified_item
        #default_impl
    };

    Ok(expanded)
}

/// Extract supported options from `#[track(...)]`.
fn extract_track_options(attrs: &[Attribute]) -> Result<TrackOptions> {
    let mut options = TrackOptions::default();
    for attr in attrs {
        if attr.path().is_ident("track") {
            if let Meta::List(meta_list) = &attr.meta {
                let metas =
                    meta_list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;

                for meta in metas {
                    if let Meta::NameValue(content) = &meta {
                        if content.path.is_ident("name") {
                            if let Expr::Lit(expr_lit) = &content.value {
                                if let Lit::Str(s) = &expr_lit.lit {
                                    options.name = Some(s.value());
                                    continue;
                                }
                            }
                            return Err(Error::new_spanned(
                                &content.value,
                                "#[track(name = ...)] requires a string literal",
                            ));
                        }

                        if content.path.is_ident("permits") {
                            if let Expr::Lit(expr_lit) = &content.value {
                                if let Lit::Int(i) = &expr_lit.lit {
                                    options.permits = Some(i.base10_parse::<usize>()?);
                                    continue;
                                }
                            }
                            return Err(Error::new_spanned(
                                &content.value,
                                "#[track(permits = ...)] requires an integer literal",
                            ));
                        }
                    }

                    return Err(Error::new_spanned(
                        meta,
                        "unsupported #[track] argument; expected name = \"...\" or permits = N",
                    ));
                }
            }
        }
    }
    Ok(options)
}

/// Map a field type (`Mutex<T>`, `RwLock<T>`, etc.) to its `Tracked*` equivalent
/// and generate Default code.
fn map_field_type_to_tracked(
    field_type: &Type,
    resource_name: &str,
    track_options: &TrackOptions,
) -> Result<(Type, proc_macro2::TokenStream)> {
    // Try to extract the type name and generic args
    if let Type::Path(type_path) = field_type {
        let path_str = typepath_to_string(&type_path.path);

        // Check for std::sync:: prefixed types
        if path_str.contains("std::sync::Mutex") || path_str.contains("::std::sync::Mutex") {
            if let Some(inner) = extract_first_generic(&type_path.path) {
                let tracked_type: Type = syn::parse_str(&format!(
                    "::laplace_sdk::__macro_support::TrackedStdMutex<{}>",
                    type_to_string(inner)
                ))?;
                let default_code = quote! {
                    ::laplace_sdk::__macro_support::TrackedStdMutex::new(
                        <#inner as ::std::default::Default>::default(),
                        #resource_name
                    )
                };
                return Ok((tracked_type, default_code));
            }
        }

        if path_str.contains("std::sync::RwLock") || path_str.contains("::std::sync::RwLock") {
            if let Some(inner) = extract_first_generic(&type_path.path) {
                let tracked_type: Type = syn::parse_str(&format!(
                    "::laplace_sdk::__macro_support::TrackedStdRwLock<{}>",
                    type_to_string(inner)
                ))?;
                let default_code = quote! {
                    ::laplace_sdk::__macro_support::TrackedStdRwLock::new(
                        <#inner as ::std::default::Default>::default(),
                        #resource_name
                    )
                };
                return Ok((tracked_type, default_code));
            }
        }

        // Check for tokio Mutex/RwLock (without std::sync:: prefix)
        if path_str.ends_with("Mutex") {
            if let Some(inner) = extract_first_generic(&type_path.path) {
                let tracked_type: Type = syn::parse_str(&format!(
                    "::laplace_sdk::__macro_support::TrackedMutex<{}>",
                    type_to_string(inner)
                ))?;
                let default_code = quote! {
                    ::laplace_sdk::__macro_support::TrackedMutex::new(
                        <#inner as ::std::default::Default>::default(),
                        #resource_name
                    )
                };
                return Ok((tracked_type, default_code));
            }
        }

        if path_str.ends_with("RwLock") && !path_str.contains("std::sync") {
            if let Some(inner) = extract_first_generic(&type_path.path) {
                let tracked_type: Type = syn::parse_str(&format!(
                    "::laplace_sdk::__macro_support::TrackedRwLock<{}>",
                    type_to_string(inner)
                ))?;
                let default_code = quote! {
                    ::laplace_sdk::__macro_support::TrackedRwLock::new(
                        <#inner as ::std::default::Default>::default(),
                        #resource_name
                    )
                };
                return Ok((tracked_type, default_code));
            }
        }

        // Atomic types (no generic parameters)
        if path_str.ends_with("AtomicBool") {
            let tracked_type: Type =
                syn::parse_str("::laplace_sdk::__macro_support::TrackedAtomicBool")?;
            let default_code = quote! {
                ::laplace_sdk::__macro_support::TrackedAtomicBool::new(false, #resource_name)
            };
            return Ok((tracked_type, default_code));
        }

        if path_str.ends_with("AtomicU32") {
            let tracked_type: Type =
                syn::parse_str("::laplace_sdk::__macro_support::TrackedAtomicU32")?;
            let default_code = quote! {
                ::laplace_sdk::__macro_support::TrackedAtomicU32::new(0, #resource_name)
            };
            return Ok((tracked_type, default_code));
        }

        if path_str.ends_with("AtomicU64") {
            let tracked_type: Type =
                syn::parse_str("::laplace_sdk::__macro_support::TrackedAtomicU64")?;
            let default_code = quote! {
                ::laplace_sdk::__macro_support::TrackedAtomicU64::new(0, #resource_name)
            };
            return Ok((tracked_type, default_code));
        }

        if path_str.ends_with("AtomicUsize") {
            let tracked_type: Type =
                syn::parse_str("::laplace_sdk::__macro_support::TrackedAtomicUsize")?;
            let default_code = quote! {
                ::laplace_sdk::__macro_support::TrackedAtomicUsize::new(0, #resource_name)
            };
            return Ok((tracked_type, default_code));
        }

        if path_str.ends_with("Semaphore") {
            let tracked_type: Type =
                syn::parse_str("::laplace_sdk::__macro_support::TrackedSemaphore")?;
            let Some(permits) = track_options.permits else {
                return Err(Error::new_spanned(
                    field_type,
                    "Semaphore fields require #[track(permits = N)]",
                ));
            };
            let default_code = quote! {
                ::laplace_sdk::__macro_support::TrackedSemaphore::new(#permits, #resource_name)
            };
            return Ok((tracked_type, default_code));
        }
    }

    Err(Error::new_spanned(
        field_type,
        format!(
            "unsupported type for #[track]: {}",
            type_to_string(field_type)
        ),
    ))
}

/// Convert a `syn::Path` to a string representation.
fn typepath_to_string(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|seg| seg.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

/// Convert any `syn::Type` to string.
fn type_to_string(ty: &Type) -> String {
    quote!(#ty).to_string()
}

/// Extract the first generic argument from a type path.
fn extract_first_generic(path: &syn::Path) -> Option<&Type> {
    let last_seg = path.segments.last()?;
    if let PathArguments::AngleBracketed(ab) = &last_seg.arguments {
        if let Some(GenericArgument::Type(inner)) = ab.args.first() {
            return Some(inner);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::expand_attribute;
    use quote::quote;
    use syn::{parse_quote, ItemStruct};

    #[test]
    fn default_impl_preserves_struct_generics() {
        let item: ItemStruct = parse_quote! {
            struct Service<T: Default> {
                #[track]
                lock: tokio::sync::Mutex<u64>,
                extra: T,
            }
        };

        let expanded = expand_attribute(quote! {}, item)
            .expect("tracked expansion succeeds")
            .to_string();

        assert!(
            expanded
                .contains("impl < T : Default > :: std :: default :: Default for Service < T >"),
            "{expanded}"
        );
    }

    #[test]
    fn semaphore_default_uses_declared_permits() {
        let item: ItemStruct = parse_quote! {
            struct Gate {
                #[track(permits = 2)]
                limiter: tokio::sync::Semaphore,
            }
        };

        let expanded = expand_attribute(quote! {}, item)
            .expect("tracked expansion succeeds")
            .to_string();

        assert!(
            expanded.contains("TrackedSemaphore :: new (2usize , \"limiter\")"),
            "{expanded}"
        );
    }

    #[test]
    fn semaphore_without_permits_is_rejected() {
        let item: ItemStruct = parse_quote! {
            struct Gate {
                #[track]
                limiter: tokio::sync::Semaphore,
            }
        };

        let err = expand_attribute(quote! {}, item).expect_err("missing permits rejects");
        assert!(
            err.to_string()
                .contains("Semaphore fields require #[track(permits = N)]"),
            "{err}"
        );
    }
}
