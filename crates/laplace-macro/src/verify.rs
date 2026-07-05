// SPDX-License-Identifier: Apache-2.0
//! `#[laplace::verify]` — improved Ki-DPOR verification harness.
//!
//! The `#[laplace::verify(threads = N)]` attribute is an enhanced version of
//! `#[axiom_target]` that supports `&T` references (in addition to `Arc<T>`),
//! includes zero-event warnings, and is more configurable.

use proc_macro::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream, Result};
use syn::punctuated::Punctuated;
use syn::{Expr, ItemFn, Lit, Meta, Token};

const VALID_VERIFY_KEYS: &str =
    "threads, name, expected, determinism, write_ard, output_dir, buffer, max_depth";

const VALID_DETERMINISM_LABELS: &[&str] = &[
    "fully_deterministic",
    "fully-deterministic",
    "full",
    "deterministic_with_declared_inputs",
    "deterministic-with-declared-inputs",
    "declared_inputs",
    "declared",
];

/// Parsed arguments from `#[laplace::verify(...)]`.
pub(crate) struct VerifyArgs {
    pub(crate) threads: usize,
    pub(crate) name: Option<String>,
    pub(crate) expected: Option<String>,
    pub(crate) determinism: String,
    pub(crate) write_ard: bool,
    pub(crate) output_dir: String,
    pub(crate) buffer: usize,
    pub(crate) max_depth: Option<usize>,
}

impl Parse for VerifyArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut threads = None;
        let mut name = None;
        let mut expected = None;
        let mut determinism = None;
        let mut write_ard = None;
        let mut output_dir = None;
        let mut buffer = None;
        let mut max_depth = None;

        let metas = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;
        for meta in metas {
            let Meta::NameValue(nv) = meta else {
                return Err(syn::Error::new_spanned(
                    meta,
                    format!(
                        "expected `key = value` argument for `#[laplace_sdk::verify]`; valid keys: {VALID_VERIFY_KEYS}"
                    ),
                ));
            };

            let key = nv.path.get_ident().map(|i| i.to_string());
            let literal = match &nv.value {
                Expr::Lit(expr_lit) => &expr_lit.lit,
                _ => {
                    let key_name = key.as_deref().unwrap_or("<unknown>");
                    return Err(syn::Error::new_spanned(
                        &nv.value,
                        format!("expected literal value for `{key_name}`"),
                    ));
                }
            };

            match key.as_deref() {
                Some("threads") => {
                    let Lit::Int(i) = literal else {
                        return Err(syn::Error::new_spanned(
                            literal,
                            "expected integer literal for `threads`",
                        ));
                    };
                    let value = i.base10_parse::<usize>()?;
                    if value == 0 {
                        return Err(syn::Error::new_spanned(
                            i,
                            "`threads` must be between 1 and 8",
                        ));
                    }
                    if value > 8 {
                        return Err(syn::Error::new_spanned(i, "`threads` must not exceed 8"));
                    }
                    threads = Some(value);
                }
                Some("name") => {
                    let Lit::Str(s) = literal else {
                        return Err(syn::Error::new_spanned(
                            literal,
                            "expected string literal for `name`",
                        ));
                    };
                    name = Some(s.value());
                }
                Some("expected") => {
                    let Lit::Str(s) = literal else {
                        return Err(syn::Error::new_spanned(
                            literal,
                            "expected string literal for `expected`",
                        ));
                    };
                    let value = s.value();
                    if value != "clean" && value != "bug" {
                        return Err(syn::Error::new_spanned(
                            s,
                            "unsupported `expected` value; expected \"clean\" or \"bug\"",
                        ));
                    }
                    expected = Some(value);
                }
                Some("determinism") => {
                    let Lit::Str(s) = literal else {
                        return Err(syn::Error::new_spanned(
                            literal,
                            "expected string literal for `determinism`",
                        ));
                    };
                    let value = s.value();
                    if !VALID_DETERMINISM_LABELS.contains(&value.as_str()) {
                        return Err(syn::Error::new_spanned(
                            s,
                            "unsupported `determinism` value; expected one of: fully_deterministic, fully-deterministic, full, deterministic_with_declared_inputs, deterministic-with-declared-inputs, declared_inputs, declared",
                        ));
                    }
                    determinism = Some(value);
                }
                Some("write_ard") => {
                    let Lit::Bool(b) = literal else {
                        return Err(syn::Error::new_spanned(
                            literal,
                            "expected boolean literal for `write_ard`",
                        ));
                    };
                    write_ard = Some(b.value());
                }
                Some("output_dir") => {
                    let Lit::Str(s) = literal else {
                        return Err(syn::Error::new_spanned(
                            literal,
                            "expected string literal for `output_dir`",
                        ));
                    };
                    output_dir = Some(s.value());
                }
                Some("buffer") => {
                    let Lit::Int(i) = literal else {
                        return Err(syn::Error::new_spanned(
                            literal,
                            "expected integer literal for `buffer`",
                        ));
                    };
                    buffer = Some(i.base10_parse::<usize>()?);
                }
                Some("max_depth") => {
                    let Lit::Int(i) = literal else {
                        return Err(syn::Error::new_spanned(
                            literal,
                            "expected integer literal for `max_depth`",
                        ));
                    };
                    max_depth = Some(i.base10_parse::<usize>()?);
                }
                _ => {
                    let key_name = nv
                        .path
                        .segments
                        .last()
                        .map(|segment| segment.ident.to_string())
                        .unwrap_or_else(|| "<unknown>".to_string());
                    return Err(syn::Error::new_spanned(
                        &nv,
                        format!(
                            "unknown argument `{key_name}` for `#[laplace_sdk::verify]`; valid keys: {VALID_VERIFY_KEYS}"
                        ),
                    ));
                }
            }
        }

        Ok(VerifyArgs {
            threads: threads.ok_or_else(|| input.error("verify: `threads` is required"))?,
            name,
            expected,
            determinism: determinism.unwrap_or_else(|| "fully_deterministic".to_string()),
            write_ard: write_ard.unwrap_or(false),
            output_dir: output_dir.unwrap_or_else(|| ".".to_string()),
            buffer: buffer.unwrap_or(8192),
            max_depth,
        })
    }
}

/// 함수에 `#[laplace::verify(threads = N)]`을 붙이면 Ki-DPOR 검증 테스트를 자동 생성한다.
///
/// # 지원 시그니처
///
/// - `async fn test(state: &T)` — 공유 상태 참조 (권장)
/// - `async fn test(state: Arc<T>)` — 공유 상태 Arc (하위 호환)
/// - `async fn test()` — 상태 없이 각 스레드가 독립적으로 실행
///
/// # 파라미터
///
/// - `threads` (필수): 동시 스레드 수 (≤ 8)
/// - `expected` (기본: "clean"): "clean" 또는 "bug"
/// - `write_ard` (기본: false): ARD 출력 여부
/// - `output_dir` (기본: "."): 출력 디렉토리
/// - `buffer` (기본: 8192): 이벤트 채널 버퍼 크기
/// - `max_depth` (기본: None): DPOR 최대 깊이
///
/// # 예시
///
/// ```rust,ignore
/// #[laplace::verify(threads = 2, expected = "clean")]
/// async fn test_cache(state: &AppState) {
///     let mut cache = state.cache.lock().await;
///     cache.insert("key".into(), "value".into());
/// }
/// ```
pub(crate) fn laplace_verify_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    use syn::parse_macro_input;

    let args = parse_macro_input!(attr as VerifyArgs);
    let mut func = parse_macro_input!(item as ItemFn);

    if let Some(attr) = func.attrs.iter().find(|attr| is_test_attribute(attr)) {
        return syn::Error::new_spanned(
            attr,
            "#[laplace_sdk::verify] generates the test fn; remove #[test]",
        )
        .to_compile_error()
        .into();
    }

    // Single-annotation control layer: `#[laplace::verify]` self-contains the
    // model rewrite, so users no longer need a separate `#[laplace::model]`
    // line. Apply the shared model rewrite (qualified `std::thread::spawn` →
    // `::laplace_sdk::rt::spawn`, `std::sync::{Mutex,RwLock}` →
    // `::laplace_sdk::rt::{ModelMutex,ModelRwLock}`, plus un-modeled blind-spot
    // markers) to the parsed function body BEFORE generating the harness below,
    // so the emitted `#func` already carries the rewritten primitives.
    //
    // Honest limit: the rewrite only covers source-level call/type paths. The
    // `[patch.crates-io]` redirection that swaps the real concurrency crates for
    // their Laplace shims is a compile-time Cargo setting and CANNOT be injected
    // by a proc-macro; it is emitted by onboarding (`laplace init`).
    crate::model::apply_model_rewrite(&mut func);

    let func_ident = &func.sig.ident;
    let threads = args.threads;
    let target_name_expr = match args.name {
        Some(name) => quote! { #name },
        None => {
            let func_name = func_ident.to_string();
            quote! { concat!(module_path!(), "::", #func_name) }
        }
    };
    let expected_declared = args.expected.is_some();
    let expected = args.expected.as_deref().unwrap_or("clean").to_string();
    let determinism = &args.determinism;
    let write_ard = args.write_ard;
    let output_dir = &args.output_dir;
    let buffer = args.buffer;
    let max_depth = args.max_depth;

    let test_fn_name =
        syn::Ident::new(&format!("__laplace_verify_{func_ident}"), func_ident.span());

    // 첫 번째 파라미터 검사: &T, Arc<T>, 또는 없음
    enum StateSignature {
        Reference(syn::Type), // &T
        Arc(syn::Type),       // Arc<T>
        None,
    }

    fn classify_state_type(ty: &syn::Type) -> Option<StateSignature> {
        if let syn::Type::Reference(type_ref) = ty {
            return Some(StateSignature::Reference((*type_ref.elem).clone()));
        }

        let syn::Type::Path(type_path) = ty else {
            return None;
        };
        let seg = type_path.path.segments.last()?;
        if seg.ident != "Arc" {
            return None;
        }
        let syn::PathArguments::AngleBracketed(ab) = &seg.arguments else {
            return None;
        };
        if ab.args.len() != 1 {
            return None;
        }
        match ab.args.first()? {
            syn::GenericArgument::Type(inner) => Some(StateSignature::Arc(inner.clone())),
            _ => None,
        }
    }

    let unsupported_signature_msg =
        "unsupported `#[laplace_sdk::verify]` function signature; supported signatures are `fn name()`, `fn name(state: &T)`, or `fn name(state: Arc<T>)` (sync or async)";

    let mut inputs = func.sig.inputs.iter();
    let first_input = inputs.next();
    if inputs.next().is_some() {
        return syn::Error::new_spanned(&func.sig.inputs, unsupported_signature_msg)
            .to_compile_error()
            .into();
    }

    let state_signature = match first_input {
        None => StateSignature::None,
        Some(syn::FnArg::Receiver(receiver)) => {
            return syn::Error::new_spanned(receiver, unsupported_signature_msg)
                .to_compile_error()
                .into();
        }
        Some(syn::FnArg::Typed(pat_type)) => {
            if let Some(signature) = classify_state_type(&pat_type.ty) {
                signature
            } else {
                return syn::Error::new_spanned(&pat_type.ty, unsupported_signature_msg)
                    .to_compile_error()
                    .into();
            }
        }
    };

    let is_async = func.sig.asyncness.is_some();
    let (state_init, state_clone, state_pass) = match (state_signature, is_async) {
        (StateSignature::Reference(st), true) => {
            let state_init = quote! {
                let state = ::std::sync::Arc::new(<#st as ::std::default::Default>::default());
            };
            let state_clone = quote! {
                let s = state.clone();
            };
            let state_pass = quote! {
                let rt = ::laplace_sdk::__macro_support::tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("laplace verify: tokio runtime build failed");
                rt.block_on(#func_ident(&*s));
            };
            (state_init, state_clone, state_pass)
        }
        (StateSignature::Reference(st), false) => {
            let state_init = quote! {
                let state = ::std::sync::Arc::new(<#st as ::std::default::Default>::default());
            };
            let state_clone = quote! {
                let s = state.clone();
            };
            let state_pass = quote! {
                #func_ident(&*s);
            };
            (state_init, state_clone, state_pass)
        }
        (StateSignature::Arc(st), true) => {
            let state_init = quote! {
                let state = ::std::sync::Arc::new(<#st as ::std::default::Default>::default());
            };
            let state_clone = quote! {
                let s = state.clone();
            };
            let state_pass = quote! {
                let rt = ::laplace_sdk::__macro_support::tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("laplace verify: tokio runtime build failed");
                rt.block_on(#func_ident(s));
            };
            (state_init, state_clone, state_pass)
        }
        (StateSignature::Arc(st), false) => {
            let state_init = quote! {
                let state = ::std::sync::Arc::new(<#st as ::std::default::Default>::default());
            };
            let state_clone = quote! {
                let s = state.clone();
            };
            let state_pass = quote! {
                #func_ident(s);
            };
            (state_init, state_clone, state_pass)
        }
        (StateSignature::None, true) => {
            let state_init = quote! {};
            let state_clone = quote! {};
            let state_pass = quote! {
                let rt = ::laplace_sdk::__macro_support::tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("laplace verify: tokio runtime build failed");
                rt.block_on(#func_ident());
            };
            (state_init, state_clone, state_pass)
        }
        (StateSignature::None, false) => {
            let state_init = quote! {};
            let state_clone = quote! {};
            let state_pass = quote! {
                #func_ident();
            };
            (state_init, state_clone, state_pass)
        }
    };

    let max_depth_config = if let Some(md) = max_depth {
        quote! {
            max_depth: #md,
        }
    } else {
        quote! {}
    };

    let _ = buffer; // legacy `buffer` arg is a no-op under the unbounded session sink

    let expanded = quote! {
        // 원본 함수 — 변경 없이 보존
        #func

        // 생성된 검증 테스트
        #[cfg(test)]
        #[test]
        #[allow(non_snake_case)]
        fn #test_fn_name() {
            use ::laplace_sdk::__macro_support::{
                set_probe_thread_id,
                CaptureSession,
                ProbeSessionConfig,
                ProbeEvent,
                run_verification_from,
            };

            // 1. 스코프드 캡처 세션 시작.
            //    프로세스 전역 배타(병렬 테스트 교차 오염 차단) + unbounded 싱크
            //    (버퍼 데드락 없음) + 백그라운드 동시 드레인(hang 없음). 워커·모델
            //    스폰 자식 스레드는 별도 등록 없이 이 세션 싱크로 방출한다.
            let __laplace_session = CaptureSession::begin();

            // 2. 공유 상태 초기화 (스레드 루프 밖 — 모든 스레드가 공유)
            #state_init

            // 3. N개 OS 스레드 스폰
            let mut handles = Vec::new();
            for i in 0usize..#threads {
                #state_clone  // Arc::clone
                handles.push(::std::thread::spawn(move || {
                    // 논리 스레드 id만 등록 (이벤트 싱크는 세션 전역)
                    set_probe_thread_id(i as u64);

                    // 개별 tokio 런타임으로 async 함수 실행
                    let rt = ::laplace_sdk::__macro_support::tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("laplace verify: tokio runtime build failed");

                    #state_pass
                }));
            }

            // 4. 모든 스레드 완료 대기
            for h in handles {
                h.join().expect("laplace verify: verification thread panicked");
            }

            // 5. 세션 종료 → 이벤트 수집(싱크 해제 후 collector join)
            let events: Vec<ProbeEvent> = __laplace_session.finish();

            let config = ProbeSessionConfig {
                write_ard: #write_ard,
                output_dir: #output_dir.to_string(),
                #max_depth_config
                ..ProbeSessionConfig::default()
            };
            let __laplace_target_name = #target_name_expr;

            // 6. Public reference check (tier 1), not the private engine verdict.
            //    This assert is the public conservative lock-order checker over the
            //    single captured trace. Full schedule-space engine gating (tier 2)
            //    is performed by `laplace axiom verify`.
            let __laplace_expected = #expected;
            if events.is_empty() {
                if #expected_declared {
                    panic!(
                        "laplace verify: 0 events + declared expected -- vacuous verdict blocked for '{}'. \
                         Check that TrackedMutex/RwLock instrumentation is wired.",
                        __laplace_target_name
                    );
                } else {
                    eprintln!(
                        "[laplace] WARNING: 0 events collected for '{}'. \
                         Check that TrackedMutex/RwLock are being used.",
                        __laplace_target_name
                    );
                }
            } else {
                let __laplace_reference =
                    run_verification_from(&events, __laplace_target_name, &config);
                match __laplace_expected {
                    "clean" => __laplace_reference.assert_clean(),
                    "bug" => __laplace_reference.assert_bug(),
                    other => panic!(
                        "laplace verify: unsupported expected value '{}' for '{}'; expected \"clean\" or \"bug\"",
                        other,
                        __laplace_target_name
                    ),
                }
            }

            // 7. Public macro output collects trace data only. Commercial
            //    engine verification runs through the private CLI/API boundary: when
            //    `$LAPLACE_VERIFY_EVENTS_DIR` is set the captured trace (with the
            //    declared expectation) is dumped as `<target>.json` for
            //    `laplace axiom verify --model-events <dir>` to drive the engine.
            //    No-op under a plain `cargo test`.
            ::laplace_sdk::__macro_support::dump_events_if_configured(
                __laplace_target_name, __laplace_expected, #determinism, &events,
            );
        }
    };

    TokenStream::from(expanded)
}

fn is_test_attribute(attr: &syn::Attribute) -> bool {
    let path = attr.path();
    path.is_ident("test")
        || (path.segments.len() == 2
            && path.segments[0].ident == "tokio"
            && path.segments[1].ident == "test")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(input: &str) -> VerifyArgs {
        syn::parse_str(input).expect("verify args should parse")
    }

    #[test]
    fn expected_is_none_when_not_declared() {
        let args = parse_args("threads = 2");

        assert_eq!(args.expected, None);
    }

    #[test]
    fn expected_preserves_declared_value() {
        let args = parse_args(r#"threads = 2, expected = "bug""#);

        assert_eq!(args.expected.as_deref(), Some("bug"));
    }
}
