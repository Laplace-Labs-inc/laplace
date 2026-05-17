//! `#[laplace_tracked]` — attribute macro for automatic Tracked* type substitution.
//!
//! Transforms fields with `#[track]` attributes from standard sync primitives
//! (Mutex, RwLock, Atomic*, Semaphore) to their Tracked* equivalents, and
//! generates a `Default` impl that instantiates each tracked field with
//! appropriate resource names.

use quote::quote;
use std::collections::HashSet;
use syn::{
    Attribute, Error, Expr, Fields, GenericArgument, ItemStruct, Meta, PathArguments, Result, Type,
};

/// Expand `#[laplace_tracked]` attribute macro.
pub(crate) fn expand_attribute(
    attr: proc_macro2::TokenStream,
    item: ItemStruct,
) -> Result<proc_macro2::TokenStream> {
    // For now, ignore any arguments (reserved for future use)
    let _ = attr;

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
                // Parse #[track] for optional name override
                let track_name = extract_track_name(&field.attrs)?;
                let resource_name = track_name.unwrap_or_else(|| field_name_str.clone());

                // Check for duplicate resource names
                if !resource_names.insert(resource_name.clone()) {
                    return Err(Error::new_spanned(
                        field,
                        format!("duplicate resource name: '{}'", resource_name),
                    ));
                }

                // Map field type to Tracked* type
                let (tracked_type, default_code) =
                    map_field_type_to_tracked(&field.ty, &resource_name)?;

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
    let default_impl = quote! {
        impl ::std::default::Default for #struct_name {
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

/// Extract the `name = "..."` attribute from `#[track(...)]`.
fn extract_track_name(attrs: &[Attribute]) -> Result<Option<String>> {
    for attr in attrs {
        if attr.path().is_ident("track") {
            if let Meta::List(meta_list) = &attr.meta {
                // #[track(...)] — parse as NameValue
                let content = meta_list.parse_args::<syn::MetaNameValue>()?;
                if content.path.is_ident("name") {
                    if let Expr::Lit(expr_lit) = &content.value {
                        if let syn::Lit::Str(s) = &expr_lit.lit {
                            return Ok(Some(s.value()));
                        }
                    }
                }
            }
        }
    }
    Ok(None)
}

/// Map a field type (Mutex<T>, RwLock<T>, etc.) to its Tracked* equivalent
/// and generate Default code.
fn map_field_type_to_tracked(
    field_type: &Type,
    resource_name: &str,
) -> Result<(Type, proc_macro2::TokenStream)> {
    // Try to extract the type name and generic args
    if let Type::Path(type_path) = field_type {
        let path_str = typepath_to_string(&type_path.path);

        // Check for std::sync:: prefixed types
        if path_str.contains("std::sync::Mutex") || path_str.contains("::std::sync::Mutex") {
            if let Some(inner) = extract_first_generic(&type_path.path) {
                let tracked_type: Type = syn::parse_str(&format!(
                    "::laplace_sdk::TrackedStdMutex<{}>",
                    type_to_string(inner)
                ))?;
                let default_code = quote! {
                    ::laplace_sdk::TrackedStdMutex::new(
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
                    "::laplace_sdk::TrackedStdRwLock<{}>",
                    type_to_string(inner)
                ))?;
                let default_code = quote! {
                    ::laplace_sdk::TrackedStdRwLock::new(
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
                    "::laplace_sdk::TrackedMutex<{}>",
                    type_to_string(inner)
                ))?;
                let default_code = quote! {
                    ::laplace_sdk::TrackedMutex::new(
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
                    "::laplace_sdk::TrackedRwLock<{}>",
                    type_to_string(inner)
                ))?;
                let default_code = quote! {
                    ::laplace_sdk::TrackedRwLock::new(
                        <#inner as ::std::default::Default>::default(),
                        #resource_name
                    )
                };
                return Ok((tracked_type, default_code));
            }
        }

        // Atomic types (no generic parameters)
        if path_str.ends_with("AtomicBool") {
            let tracked_type: Type = syn::parse_str("::laplace_sdk::TrackedAtomicBool")?;
            let default_code = quote! {
                ::laplace_sdk::TrackedAtomicBool::new(false, #resource_name)
            };
            return Ok((tracked_type, default_code));
        }

        if path_str.ends_with("AtomicU32") {
            let tracked_type: Type = syn::parse_str("::laplace_sdk::TrackedAtomicU32")?;
            let default_code = quote! {
                ::laplace_sdk::TrackedAtomicU32::new(0, #resource_name)
            };
            return Ok((tracked_type, default_code));
        }

        if path_str.ends_with("AtomicU64") {
            let tracked_type: Type = syn::parse_str("::laplace_sdk::TrackedAtomicU64")?;
            let default_code = quote! {
                ::laplace_sdk::TrackedAtomicU64::new(0, #resource_name)
            };
            return Ok((tracked_type, default_code));
        }

        if path_str.ends_with("AtomicUsize") {
            let tracked_type: Type = syn::parse_str("::laplace_sdk::TrackedAtomicUsize")?;
            let default_code = quote! {
                ::laplace_sdk::TrackedAtomicUsize::new(0, #resource_name)
            };
            return Ok((tracked_type, default_code));
        }

        if path_str.ends_with("Semaphore") {
            let tracked_type: Type = syn::parse_str("::laplace_sdk::TrackedSemaphore")?;
            let default_code = quote! {
                ::laplace_sdk::TrackedSemaphore::new(0, #resource_name)
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

/// Convert a syn::Path to a string representation.
fn typepath_to_string(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|seg| seg.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

/// Convert any syn::Type to string.
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
