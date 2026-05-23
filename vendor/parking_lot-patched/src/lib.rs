// Copyright 2016 Amanieu d'Antras
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

//! This library provides implementations of `Mutex`, `RwLock`, `Condvar` and
//! `Once` that are smaller, faster and more flexible than those in the Rust
//! standard library. It also provides a `ReentrantMutex` type.

#![warn(missing_docs)]
#![warn(rust_2018_idioms)]

mod condvar;
mod elision;
mod fair_mutex;
mod mutex;
mod once;
mod raw_fair_mutex;
mod raw_mutex;
mod raw_rwlock;
mod remutex;
mod rwlock;
mod util;

// ── [LAPLACE PATCH] ────────────────────────────────────────────────────────
// feature = "laplace" 활성 시 공개 API의 Mutex/RwLock을 Tracked 래퍼로 교체한다.
// 내부 raw lock / condvar 구현은 원본을 유지해 컴파일/행동 안정성을 보존한다.
// ────────────────────────────────────────────────────────────────────────────
#[cfg(feature = "laplace")]
pub(crate) mod laplace_rwlock {
    use laplace_probe_sdk::{
        TrackedStdRwLock as TrackedRwLock, TrackedStdRwLockReadGuard as TrackedRwLockReadGuard,
        TrackedStdRwLockWriteGuard as TrackedRwLockWriteGuard,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    static RWLOCK_COUNTER: AtomicUsize = AtomicUsize::new(0);

    pub struct RwLock<T> {
        inner: TrackedRwLock<T>,
    }

    pub type RwLockReadGuard<'a, T> = TrackedRwLockReadGuard<'a, T>;
    pub type RwLockWriteGuard<'a, T> = TrackedRwLockWriteGuard<'a, T>;
    pub type RwLockUpgradableReadGuard<'a, T> = TrackedRwLockWriteGuard<'a, T>;
    pub type MappedRwLockReadGuard<'a, T> = TrackedRwLockReadGuard<'a, T>;
    pub type MappedRwLockWriteGuard<'a, T> = TrackedRwLockWriteGuard<'a, T>;

    impl<T> RwLock<T> {
        pub fn new(value: T) -> Self {
            let id = RWLOCK_COUNTER.fetch_add(1, Ordering::Relaxed);
            let name: &'static str = Box::leak(format!("parking_lot_rwlock_{id}").into_boxed_str());
            Self {
                inner: TrackedRwLock::new(value, name),
            }
        }

        pub fn read(&self) -> RwLockReadGuard<'_, T> {
            self.inner.read()
        }

        pub fn write(&self) -> RwLockWriteGuard<'_, T> {
            self.inner.write()
        }

        pub fn try_read(&self) -> Option<RwLockReadGuard<'_, T>> {
            Some(self.read())
        }

        pub fn try_write(&self) -> Option<RwLockWriteGuard<'_, T>> {
            Some(self.write())
        }

        pub fn upgradable_read(&self) -> RwLockUpgradableReadGuard<'_, T> {
            self.write()
        }
    }

    pub fn const_rwlock<T>(val: T) -> RwLock<T> {
        RwLock::new(val)
    }
}

#[cfg(feature = "laplace")]
pub(crate) mod laplace_mutex {
    use laplace_probe_sdk::{TrackedStdGuard, TrackedStdMutex};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static MUTEX_COUNTER: AtomicUsize = AtomicUsize::new(0);

    pub struct Mutex<T> {
        inner: TrackedStdMutex<T>,
    }

    pub type MutexGuard<'a, T> = TrackedStdGuard<'a, T>;
    pub type MappedMutexGuard<'a, T> = TrackedStdGuard<'a, T>;

    impl<T> Mutex<T> {
        pub fn new(value: T) -> Self {
            let id = MUTEX_COUNTER.fetch_add(1, Ordering::Relaxed);
            let name: &'static str = Box::leak(format!("parking_lot_mutex_{id}").into_boxed_str());
            Self {
                inner: TrackedStdMutex::new(value, name),
            }
        }

        pub fn lock(&self) -> MutexGuard<'_, T> {
            self.inner.lock()
        }

        pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
            Some(self.lock())
        }
    }

    pub fn const_mutex<T>(val: T) -> Mutex<T> {
        Mutex::new(val)
    }
}

#[cfg(feature = "deadlock_detection")]
pub mod deadlock;
#[cfg(not(feature = "deadlock_detection"))]
mod deadlock;

// If deadlock detection is enabled, we cannot allow lock guards to be sent to
// other threads.
#[cfg(all(feature = "send_guard", feature = "deadlock_detection"))]
compile_error!("the `send_guard` and `deadlock_detection` features cannot be used together");
#[cfg(feature = "send_guard")]
type GuardMarker = lock_api::GuardSend;
#[cfg(not(feature = "send_guard"))]
type GuardMarker = lock_api::GuardNoSend;

pub use self::condvar::{Condvar, WaitTimeoutResult};
pub use self::fair_mutex::{const_fair_mutex, FairMutex, FairMutexGuard, MappedFairMutexGuard};
#[cfg(feature = "laplace")]
pub use self::laplace_mutex::{const_mutex, MappedMutexGuard, Mutex, MutexGuard};
#[cfg(feature = "laplace")]
pub use self::laplace_rwlock::{
    const_rwlock, MappedRwLockReadGuard, MappedRwLockWriteGuard, RwLock, RwLockReadGuard,
    RwLockUpgradableReadGuard, RwLockWriteGuard,
};
#[cfg(not(feature = "laplace"))]
pub use self::mutex::{const_mutex, MappedMutexGuard, Mutex, MutexGuard};
pub use self::once::{Once, OnceState};
pub use self::raw_fair_mutex::RawFairMutex;
pub use self::raw_mutex::RawMutex;
pub use self::raw_rwlock::RawRwLock;
pub use self::remutex::{
    const_reentrant_mutex, MappedReentrantMutexGuard, RawThreadId, ReentrantMutex,
    ReentrantMutexGuard,
};
#[cfg(not(feature = "laplace"))]
pub use self::rwlock::{
    const_rwlock, MappedRwLockReadGuard, MappedRwLockWriteGuard, RwLock, RwLockReadGuard,
    RwLockUpgradableReadGuard, RwLockWriteGuard,
};
pub use ::lock_api;

#[cfg(feature = "arc_lock")]
pub use self::lock_api::{
    ArcMutexGuard, ArcReentrantMutexGuard, ArcRwLockReadGuard, ArcRwLockUpgradableReadGuard,
    ArcRwLockWriteGuard,
};
