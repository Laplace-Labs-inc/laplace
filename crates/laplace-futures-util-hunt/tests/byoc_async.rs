#[cfg(all(test, feature = "laplace"))]
mod byoc_async_tests {
    use futures::lock::Mutex as FuturesMutex;
    use laplace_macro::laplace_byoc_test;
    use std::sync::Arc;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
    }

    #[laplace_byoc_test(
        name = "byoc_futures_mutex_starvation",
        write_ard = true,
        buffer = 16384
    )]
    fn test_byoc_futures_mutex_starvation() {
        rt().block_on(async {
            let mutex = Arc::new(FuturesMutex::new(0u64));
            let results = Arc::new(std::sync::Mutex::new(Vec::<u64>::new()));
            let mut handles = Vec::new();

            for (id, add) in [(0u64, Some(1u64)), (1u64, None), (2u64, Some(10u64))] {
                let m = mutex.clone();
                let r = results.clone();
                handles.push(byoc_thread!(id, {
                    rt().block_on(async {
                        let mut guard = m.lock().await;
                        if let Some(v) = add {
                            *guard += v;
                        }
                        r.lock().expect("results lock").push(id);
                        drop(guard);
                    });
                }));
            }

            for h in handles {
                h.join().expect("thread panicked");
            }
        });
    }

    #[laplace_byoc_test(
        name = "byoc_futuresunordered_mutex",
        write_ard = true,
        output_dir = ".",
        buffer = 16384
    )]
    fn test_byoc_futures_unordered_mutex() {
        byoc_thread!(0, {
            use futures::StreamExt;
            rt().block_on(async {
                let mutex = Arc::new(FuturesMutex::new(0u64));
                let mut fut_queue = futures::stream::FuturesUnordered::new();
                for i in 0..3u64 {
                    let m = mutex.clone();
                    fut_queue.push(async move {
                        let mut g = m.lock().await;
                        *g += i;
                        i
                    });
                }

                let mut results = Vec::new();
                while let Some(val) = fut_queue.next().await {
                    results.push(val);
                }
                assert_eq!(results.len(), 3, "all futures must complete");
            });
        })
        .join()
        .expect("thread panicked");
    }
}
