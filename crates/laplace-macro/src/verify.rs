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

/// Parsed arguments from `#[laplace::verify(...)]`.
pub(crate) struct VerifyArgs {
    pub(crate) threads: usize,
    pub(crate) name: Option<String>,
    pub(crate) expected: String,
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
        let mut write_ard = None;
        let mut output_dir = None;
        let mut buffer = None;
        let mut max_depth = None;

        let metas = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;
        for meta in metas {
            if let Meta::NameValue(nv) = meta {
                let key = nv.path.get_ident().map(|i| i.to_string());
                match key.as_deref() {
                    Some("threads") => {
                        if let Expr::Lit(expr_lit) = &nv.value {
                            if let Lit::Int(i) = &expr_lit.lit {
                                threads = Some(i.base10_parse::<usize>()?);
                            }
                        }
                    }
                    Some("name") => {
                        if let Expr::Lit(expr_lit) = &nv.value {
                            if let Lit::Str(s) = &expr_lit.lit {
                                name = Some(s.value());
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
                    Some("write_ard") => {
                        if let Expr::Lit(expr_lit) = &nv.value {
                            if let Lit::Bool(b) = &expr_lit.lit {
                                write_ard = Some(b.value());
                            }
                        }
                    }
                    Some("output_dir") => {
                        if let Expr::Lit(expr_lit) = &nv.value {
                            if let Lit::Str(s) = &expr_lit.lit {
                                output_dir = Some(s.value());
                            }
                        }
                    }
                    Some("buffer") => {
                        if let Expr::Lit(expr_lit) = &nv.value {
                            if let Lit::Int(i) = &expr_lit.lit {
                                buffer = Some(i.base10_parse::<usize>()?);
                            }
                        }
                    }
                    Some("max_depth") => {
                        if let Expr::Lit(expr_lit) = &nv.value {
                            if let Lit::Int(i) = &expr_lit.lit {
                                max_depth = Some(i.base10_parse::<usize>()?);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(VerifyArgs {
            threads: threads.ok_or_else(|| input.error("verify: `threads` is required"))?,
            name,
            expected: expected.unwrap_or_else(|| "clean".to_string()),
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
    let func = parse_macro_input!(item as ItemFn);

    let func_ident = &func.sig.ident;
    let threads = args.threads;
    let target_name = args.name.unwrap_or_else(|| func_ident.to_string());
    let expected = &args.expected;
    let write_ard = args.write_ard;
    let output_dir = &args.output_dir;
    let buffer = args.buffer;
    let max_depth = args.max_depth;

    let test_fn_name = syn::Ident::new(
        &format!("__laplace_verify_{}", func_ident),
        func_ident.span(),
    );

    // 첫 번째 파라미터 검사: &T, Arc<T>, 또는 없음
    enum StateSignature {
        Reference(syn::Type), // &T
        Arc(syn::Type),       // Arc<T>
        None,
    }

    let state_signature = if let Some(syn::FnArg::Typed(pat_type)) = func.sig.inputs.first() {
        // &T 검사
        if let syn::Type::Reference(type_ref) = &*pat_type.ty {
            StateSignature::Reference((*type_ref.elem).clone())
        } else if let syn::Type::Path(type_path) = &*pat_type.ty {
            // Arc<T> 검사
            if let Some(seg) = type_path.path.segments.last() {
                if seg.ident == "Arc" {
                    if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
                        if let Some(syn::GenericArgument::Type(inner)) = ab.args.first() {
                            StateSignature::Arc(inner.clone())
                        } else {
                            StateSignature::None
                        }
                    } else {
                        StateSignature::None
                    }
                } else {
                    StateSignature::None
                }
            } else {
                StateSignature::None
            }
        } else {
            StateSignature::None
        }
    } else {
        StateSignature::None
    };

    let (state_init, state_clone, state_pass) = match state_signature {
        StateSignature::Reference(st) => {
            let state_init = quote! {
                let state = ::std::sync::Arc::new(<#st as ::std::default::Default>::default());
            };
            let state_clone = quote! {
                let s = state.clone();
            };
            let state_pass = quote! {
                rt.block_on(#func_ident(&*s));
            };
            (state_init, state_clone, state_pass)
        }
        StateSignature::Arc(st) => {
            let state_init = quote! {
                let state = ::std::sync::Arc::new(<#st as ::std::default::Default>::default());
            };
            let state_clone = quote! {
                let s = state.clone();
            };
            let state_pass = quote! {
                rt.block_on(#func_ident(s));
            };
            (state_init, state_clone, state_pass)
        }
        StateSignature::None => {
            let state_init = quote! {};
            let state_clone = quote! {};
            let state_pass = quote! {
                rt.block_on(#func_ident());
            };
            (state_init, state_clone, state_pass)
        }
    };

    let max_depth_config = if let Some(md) = max_depth {
        quote! {
            max_depth: Some(#md),
        }
    } else {
        quote! {}
    };

    let expanded = quote! {
        // 원본 함수 — 변경 없이 보존
        #func

        // 생성된 Ki-DPOR 검증 테스트
        #[cfg(test)]
        #[test]
        #[allow(non_snake_case)]
        fn #test_fn_name() {
            use ::std::sync::{Arc, mpsc};
            use ::laplace_sdk::{
                set_probe_sender,
                set_probe_thread_id,
                ProbeSessionConfig,
                run_verification_from,
                ProbeEvent,
            };

            // 1. 이벤트 수집 채널
            let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(#buffer);

            // 2. 공유 상태 초기화 (스레드 루프 밖 — 모든 스레드가 공유)
            #state_init

            // 3. N개 OS 스레드 스폰
            let mut handles = Vec::new();
            for i in 0usize..#threads {
                let tx2 = tx.clone();
                #state_clone  // Arc::clone
                handles.push(::std::thread::spawn(move || {
                    // 각 OS 스레드에서 thread-local 초기화
                    set_probe_sender(tx2);
                    set_probe_thread_id(i as u64);

                    // 개별 tokio 런타임으로 async 함수 실행
                    let rt = ::tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("laplace verify: tokio runtime build failed");

                    #state_pass
                }));
            }

            // 4. 송신단 drop → 채널 종료 신호
            drop(tx);

            // 5. 모든 스레드 완료 대기
            for h in handles {
                h.join().expect("laplace verify: verification thread panicked");
            }

            // 6. 이벤트 수집
            let events: Vec<ProbeEvent> = rx.into_iter().collect();

            // 7. 이벤트 0건 경고 (Silent CLEAN 방지)
            if events.is_empty() {
                eprintln!(
                    "[laplace] WARNING: 0 events collected for '{}'. \
                     Check that TrackedMutex/RwLock are being used.",
                    #target_name
                );
            }

            // 8. Ki-DPOR 실행 + 결과 검증
            let config = ProbeSessionConfig {
                write_ard: #write_ard,
                output_dir: #output_dir.to_string(),
                #max_depth_config
                ..ProbeSessionConfig::default()
            };

            let result = run_verification_from(&events, #target_name, &config);

            // 9. expected 파라미터에 따라 assert
            if #expected == "bug" {
                result.assert_bug();
            } else {
                result.assert_clean();
            }
        }
    };

    TokenStream::from(expanded)
}
