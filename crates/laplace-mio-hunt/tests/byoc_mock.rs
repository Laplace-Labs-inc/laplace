//! BYOC — MockPipeInner로 mio 패턴 재현
//!
//! [DX 관찰 포인트]
//! - vendor 없이 mock 전략을 쓰는 것이 BYOC를 더 쉽게 만드는가?
//! - 실제 mio 코드와 얼마나 유사한가?
//! - Windows 전용 코드를 Linux에서 테스트하는 mock 전략의 한계는?

#[cfg(test)]
#[cfg(feature = "laplace")]
mod byoc_mock_tests {
    use laplace_macro::laplace_byoc_test;
    use laplace_mio_hunt::mock::MockPipeInner;

    /// BYOC 시나리오 1: 두 스레드가 동시에 write_with_buffer + read_and_recycle
    /// 실제 mio 코드 패턴 재현: io → pool 일관 순서
    #[laplace_byoc_test(
        name = "byoc_mio_consistent_ordering",
        write_ard = true,
        lock_events_only = true
    )]
    fn test_byoc_consistent_io_pool() {
        let inner = MockPipeInner::new();

        let i0 = inner.clone();
        let h0 = byoc_thread!(0, {
            i0.write_with_buffer(42);
        });

        let i1 = inner.clone();
        let h1 = byoc_thread!(1, {
            let _ = i1.read_and_recycle();
        });

        h0.join().expect("thread panicked");
        h1.join().expect("thread panicked");
    }

    /// BYOC 시나리오 2: 두 스레드 동시 connect() — connecting 플래그 경합
    #[laplace_byoc_test(
        name = "byoc_mio_concurrent_connect",
        write_ard = true,
        output_dir = ".",
        lock_events_only = true
    )]
    fn test_byoc_concurrent_connect() {
        let inner = MockPipeInner::new();

        let mut handles = Vec::new();
        for thread_id in 0..2u64 {
            let i = inner.clone();
            handles.push(byoc_thread!(thread_id, {
                let _ = i.connect();
            }));
        }

        for h in handles {
            h.join().expect("thread panicked");
        }
    }
}
