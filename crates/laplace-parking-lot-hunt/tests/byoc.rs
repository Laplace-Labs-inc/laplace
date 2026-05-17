#[cfg(all(test, feature = "laplace"))]
mod byoc_tests {
    use laplace_macro::laplace_byoc_test;
    use parking_lot::RwLock;
    use std::sync::Arc;

    #[laplace_byoc_test(name = "byoc_two_rwlocks_abba", expected = "bug", write_ard = true)]
    fn test_byoc_two_rwlocks_abba() {
        struct CacheService {
            entries: RwLock<Vec<String>>,
            metadata: RwLock<Vec<u64>>,
        }

        let svc = Arc::new(CacheService {
            entries: RwLock::new(vec![]),
            metadata: RwLock::new(vec![]),
        });

        let svc0 = svc.clone();
        byoc_thread!(0, {
            let _entries = svc0.entries.read();
            let _meta = svc0.metadata.write();
        })
        .join()
        .expect("thread panicked");

        let svc1 = svc.clone();
        byoc_thread!(1, {
            let _meta = svc1.metadata.read();
            let _entries = svc1.entries.write();
        })
        .join()
        .expect("thread panicked");
    }
}
