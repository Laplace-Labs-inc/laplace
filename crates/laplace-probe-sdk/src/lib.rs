// SPDX-License-Identifier: Apache-2.0
#![allow(clippy::all, clippy::pedantic)]

//! Laplace Probe SDK — BYOC Phase 1 매크로 시스템 지원 크레이트.
//!
//! # 사용자 공개 API
//!
//! ```ignore
//! use laplace_probe_sdk::{TrackedMutex, ProbeSessionConfig, run_verification_from};
//! ```
//!
//! # 내부 API (생성 코드 전용)
//!
//! ```ignore
//! use laplace_probe_sdk::{set_probe_sender, set_probe_thread_id};
//! ```

pub use laplace_probe::ProbeEvent;

#[cfg(feature = "cloud")]
pub mod client;
pub mod config;
pub mod license;
pub mod session;
pub mod tracked;
pub mod tracked_atomic;
#[cfg(feature = "parking_lot_tracking")]
pub mod tracked_parking_lot_rwlock;
pub mod tracked_rwlock;
pub mod tracked_semaphore;
pub mod tracked_std;
pub mod tracked_std_rwlock;

// ── 공개 재내보내기 ────────────────────────────────────────────────────────────

pub use config::{load_project_config, load_toml_max_depth, ProjectConfig};
pub use session::{
    clear_probe_sender, current_thread_id, emit, set_probe_sender, set_probe_thread_id,
    ProbeSessionConfig, VerifyResult,
};
pub use tracked::{TrackedGuard, TrackedMutex};
pub use tracked_atomic::{
    TrackedAtomicBool, TrackedAtomicU32, TrackedAtomicU64, TrackedAtomicUsize,
};
#[cfg(feature = "parking_lot_tracking")]
pub use tracked_parking_lot_rwlock::{
    TrackedParkingLotRwLock, TrackedParkingLotRwLockReadGuard, TrackedParkingLotRwLockWriteGuard,
};
pub use tracked_rwlock::{TrackedRwLock, TrackedRwLockReadGuard, TrackedRwLockWriteGuard};
pub use tracked_semaphore::{TrackedSemaphore, TrackedSemaphorePermit};
pub use tracked_std::{TrackedStdGuard, TrackedStdMutex};
pub use tracked_std_rwlock::{
    TrackedStdRwLock, TrackedStdRwLockReadGuard, TrackedStdRwLockWriteGuard,
};

#[cfg(feature = "verification")]
pub use session::run_verification_from;

#[cfg(feature = "cloud")]
pub use client::ProbeClientConfig;
#[cfg(feature = "cloud")]
pub use session::init_cloud_probe;

#[cfg(feature = "verification")]
pub use laplace_axiom::oracle::OracleVerdict;
