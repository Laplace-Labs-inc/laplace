//! mio NamedPipe 이중 Mutex + connecting 플래그 Ki-DPOR 검증
//!
//! 실제 코드 분석: io → pool 락 순서 일관됨 (AB-BA 없음)
//! 타겟: connecting(AtomicBool) + io(Mutex) 조합 TOCTOU 탐색

#[cfg(feature = "laplace")]
pub mod harness;
pub mod mock;
#[cfg(feature = "laplace")]
pub mod registry;
