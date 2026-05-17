//! `#[axiom_target]` — automated Ki-DPOR verification harness.
//!
//! The `#[axiom_target(threads = N)]` attribute automatically generates a test
//! that runs a function with N concurrent OS threads, each executing the function
//! body with isolated tokio runtimes. Probe events are collected and fed to Ki-DPOR
//! for exhaustive concurrency verification.

use proc_macro::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream, Result};
use syn::punctuated::Punctuated;
use syn::{Expr, ItemFn, Lit, Meta, Token};

/// Parsed arguments from `#[axiom_target(...)]`.
pub(crate) struct AxiomTargetArgs {
    pub(crate) threads: usize,
    pub(crate) name: Option<String>,
    pub(crate) write_ard: bool,
    pub(crate) output_dir: String,
}

impl Parse for AxiomTargetArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut threads = None;
        let mut name = None;
        let mut write_ard = true;
        let mut output_dir = ".".to_string();

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
                    Some("write_ard") => {
                        if let Expr::Lit(expr_lit) = &nv.value {
                            if let Lit::Bool(b) = &expr_lit.lit {
                                write_ard = b.value();
                            }
                        }
                    }
                    Some("output_dir") => {
                        if let Expr::Lit(expr_lit) = &nv.value {
                            if let Lit::Str(s) = &expr_lit.lit {
                                output_dir = s.value();
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(AxiomTargetArgs {
            threads: threads.ok_or_else(|| input.error("axiom_target: `threads` is required"))?,
            name,
            write_ard,
            output_dir,
        })
    }
}

/// 함수에 `#[axiom_target(threads = N)]`을 붙이면 Ki-DPOR 검증 테스트를 자동 생성한다.
///
/// # 요구사항
///
/// - 함수 시그니처: `async fn <name>(state: Arc<T>)` — 첫 번째 인자가 `Arc<T>` 형태여야 한다.
/// - `Arc<T>` 의 `T`가 `Default`를 구현해야 한다 — 생성 코드가 `T::default()`로 초기화한다.
/// - 사용자 크레이트 `[dev-dependencies]`에 `laplace-probe-sdk`, `laplace-macro` 추가 필요.
///
/// # 생성 테스트 함수 이름
///
/// `__laplace_axiom_<original_fn_name>` — `cargo test __laplace_axiom_<name>`으로 실행.
///
/// # 예시
///
/// ```ignore
/// #[axiom_target(threads = 3)]
/// async fn verify_counter(state: Arc<AppState>) {
///     let mut g = state.counter.lock().await;
///     *g += 1;
/// }
/// // 자동 생성: #[test] fn __laplace_axiom_verify_counter() { ... }
/// ```
pub(crate) fn axiom_target_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    use syn::parse_macro_input;

    let args = parse_macro_input!(attr as AxiomTargetArgs);
    let func = parse_macro_input!(item as ItemFn);

    let func_ident = &func.sig.ident;
    let threads = args.threads;
    let target_name = args.name.unwrap_or_else(|| func_ident.to_string());
    let write_ard = args.write_ard;
    let output_dir = &args.output_dir;

    // 생성할 테스트 함수 이름: __laplace_axiom_<fn_name>
    let test_fn_name = syn::Ident::new(
        &format!("__laplace_axiom_{}", func_ident),
        func_ident.span(),
    );

    // 첫 번째 파라미터에서 Arc 내부 타입 추출 (Arc<T> → T)
    // 추출 실패 시 컴파일 에러 대신 Unit 타입 폴백 (사용자에게 진단 필요)
    let state_type = func.sig.inputs.first().and_then(|arg| {
        if let syn::FnArg::Typed(pat_type) = arg {
            if let syn::Type::Path(type_path) = &*pat_type.ty {
                let seg = type_path.path.segments.last()?;
                if seg.ident == "Arc" {
                    if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
                        if let Some(syn::GenericArgument::Type(inner)) = ab.args.first() {
                            return Some(inner.clone());
                        }
                    }
                }
            }
            None
        } else {
            None
        }
    });

    let state_init = if let Some(st) = state_type {
        quote! {
            let state = ::std::sync::Arc::new(<#st as ::std::default::Default>::default());
        }
    } else {
        // 타입 추출 실패 시: 컴파일 에러 유도
        quote! {
            compile_error!(
                "axiom_target: first parameter must be Arc<T> where T: Default"
            );
        }
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
            use ::laplace_probe_sdk::{
                set_probe_sender,
                set_probe_thread_id,
                ProbeSessionConfig,
                run_verification_from,
            };

            // 1. 이벤트 수집 채널 (std::sync::mpsc — OS 스레드 간 안전)
            //    bounded(0): backpressure 없이 최대한 비동기적으로 수집
            let (tx, rx) = mpsc::sync_channel::<::laplace_probe_sdk::ProbeEvent>(4096);

            // 2. 공유 상태 초기화 (T::default())
            #state_init

            // 3. N개 OS 스레드 스폰 (thread-local 안전 보장)
            let mut handles = Vec::new();
            for i in 0usize..#threads {
                let s = state.clone();
                let tx2 = tx.clone();
                handles.push(::std::thread::spawn(move || {
                    // 각 OS 스레드에서 thread-local 독립 초기화
                    set_probe_sender(tx2);
                    set_probe_thread_id(i as u64);
                    // 개별 tokio 런타임으로 async 함수 실행
                    let rt = ::tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("laplace axiom_target: tokio runtime build failed");
                    rt.block_on(#func_ident(s));
                }));
            }

            // 4. 송신단 drop → 채널 종료 신호
            drop(tx);

            // 5. 모든 스레드 완료 대기
            for h in handles {
                h.join().expect("laplace axiom_target: verification thread panicked");
            }

            // 6. 이벤트 수집
            let events: Vec<::laplace_probe_sdk::ProbeEvent> = rx.into_iter().collect();

            // 7. Ki-DPOR 실행 + 결과 검증
            let config = ProbeSessionConfig {
                write_ard: #write_ard,
                output_dir: #output_dir.to_string(),
                ..ProbeSessionConfig::default()
            };
            run_verification_from(&events, #target_name, &config).assert_clean();
        }
    };

    TokenStream::from(expanded)
}
