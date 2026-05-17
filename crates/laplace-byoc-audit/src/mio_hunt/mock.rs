// SPDX-License-Identifier: Apache-2.0
//! mio NamedPipe Inner의 플랫폼 중립 모의(mock) 구현.
//! 실제 mio의 named_pipe.rs 구조를 그대로 반영하되 Windows API 제거.

#[cfg(feature = "laplace")]
use laplace_probe_sdk::{TrackedAtomicBool, TrackedStdGuard, TrackedStdMutex};
#[cfg(not(feature = "laplace"))]
use std::sync::atomic::AtomicBool as TrackedAtomicBool;
#[cfg(not(feature = "laplace"))]
use std::sync::{Mutex as TrackedStdMutex, MutexGuard as TrackedStdGuard};

use std::sync::atomic::Ordering::SeqCst;
use std::sync::Arc;

// mio named_pipe.rs의 Inner 구조 재현 (핵심 필드)
pub struct MockPipeInner {
    pub connecting: TrackedAtomicBool,
    pub io: TrackedStdMutex<IoState>,
    pub pool: TrackedStdMutex<PoolState>,
}

#[derive(Default)]
pub struct IoState {
    pub value: u64,
    pub write_pending: bool,
}

#[derive(Default)]
pub struct PoolState {
    pub buffers: Vec<Vec<u8>>,
    pub capacity: usize,
}

impl MockPipeInner {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            #[cfg(feature = "laplace")]
            connecting: TrackedAtomicBool::new(false, "mio_connecting"),
            #[cfg(not(feature = "laplace"))]
            connecting: TrackedAtomicBool::new(false),
            #[cfg(feature = "laplace")]
            io: TrackedStdMutex::new(IoState::default(), "mio_io_state"),
            #[cfg(not(feature = "laplace"))]
            io: std::sync::Mutex::new(IoState::default()),
            #[cfg(feature = "laplace")]
            pool: TrackedStdMutex::new(
                PoolState {
                    buffers: vec![],
                    capacity: 2,
                },
                "mio_buffer_pool",
            ),
            #[cfg(not(feature = "laplace"))]
            pool: std::sync::Mutex::new(PoolState {
                buffers: vec![],
                capacity: 2,
            }),
        })
    }

    #[inline]
    fn lock_io(&self) -> TrackedStdGuard<'_, IoState> {
        #[cfg(feature = "laplace")]
        {
            self.io.lock()
        }
        #[cfg(not(feature = "laplace"))]
        {
            self.io.lock().expect("io mutex poisoned")
        }
    }

    #[inline]
    fn lock_pool(&self) -> TrackedStdGuard<'_, PoolState> {
        #[cfg(feature = "laplace")]
        {
            self.pool.lock()
        }
        #[cfg(not(feature = "laplace"))]
        {
            self.pool.lock().expect("pool mutex poisoned")
        }
    }

    // mio 실제 코드와 동일한 순서: io → pool (R0 → R1)
    pub fn write_with_buffer(&self, data: u64) {
        let mut io = self.lock_io();
        io.value = data;
        let mut pool = self.lock_pool();
        pool.capacity = pool.capacity.saturating_sub(1);
    }

    // mio 실제 코드와 동일한 순서: io → pool (R0 → R1)
    pub fn read_and_recycle(&self) -> u64 {
        let io = self.lock_io();
        let val = io.value;
        let mut pool = self.lock_pool();
        pool.buffers.push(vec![0u8; 4096]);
        val
    }

    // connecting 플래그 + io 조합 (TOCTOU 가능성 탐색)
    //
    // [주의]: TrackedAtomicBool에는 swap()이 없어 compare_exchange를 사용한다.
    pub fn connect(&self) -> Result<(), &'static str> {
        if self
            .connecting
            .compare_exchange(false, true, SeqCst, SeqCst)
            .is_err()
        {
            return Err("already connecting");
        }

        // TOCTOU 윈도우: connecting=true지만 io lock 전 구간
        let mut io = self.lock_io();
        io.write_pending = true;
        drop(io);

        self.connecting.store(false, SeqCst);
        Ok(())
    }
}
