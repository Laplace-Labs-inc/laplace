#[cfg(all(test, feature = "laplace"))]
mod byoc_tests {
    use laplace_macro::laplace_byoc_test;
    use laplace_probe_sdk::TrackedAtomicUsize;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    struct MockShared {
        ref_count: TrackedAtomicUsize,
        data: Vec<u8>,
    }

    impl MockShared {
        fn new(initial_ref: usize) -> Arc<Self> {
            Arc::new(Self {
                ref_count: TrackedAtomicUsize::new(initial_ref, "bytes_refcount"),
                data: vec![0u8; 16],
            })
        }
        fn increment(&self) {
            self.ref_count.fetch_add(1, Ordering::Relaxed);
        }
        fn decrement(&self) -> bool {
            self.ref_count.fetch_sub(1, Ordering::Release) == 1
        }
        fn is_unique(&self) -> bool {
            self.ref_count.load(Ordering::Acquire) == 1
        }
    }

    #[laplace_byoc_test(name = "byoc_bytes_toctou", expected = "bug", write_ard = true)]
    fn test_byoc_is_unique_toctou() {
        let shared = MockShared::new(1);
        let s0 = shared.clone();
        let h0 = byoc_thread!(0, {
            s0.increment();
            let _ = s0.decrement();
        });
        let s1 = shared.clone();
        let h1 = byoc_thread!(1, {
            if s1.is_unique() {
                let _ = s1.is_unique();
            }
            let _ = s1.decrement();
        });
        h0.join().expect("thread panicked");
        h1.join().expect("thread panicked");
    }

    #[laplace_byoc_test(name = "byoc_bytes_single", write_ard = true, output_dir = ".")]
    fn test_byoc_single_thread_refcount() {
        byoc_thread!(0, {
            let shared = MockShared::new(1);
            assert_eq!(shared.ref_count.load(Ordering::SeqCst), 1);
            assert_eq!(shared.data.len(), 16);
            shared.increment();
            assert_eq!(shared.ref_count.load(Ordering::SeqCst), 2);
            shared.decrement();
            assert_eq!(shared.ref_count.load(Ordering::SeqCst), 1);
            assert!(shared.is_unique());
        })
        .join()
        .expect("thread panicked");
    }
}
