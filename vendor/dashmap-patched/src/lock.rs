#[allow(unused_imports)]
use core::sync::atomic::{AtomicUsize, Ordering};
#[allow(unused_imports)]
use parking_lot_core::{ParkToken, SpinWait, UnparkToken};

// [LAPLACE PATCH] feature-gated RwLock implementation
#[cfg(feature = "laplace")]
pub(crate) mod laplace_lock {
    use core::sync::atomic::{AtomicUsize, Ordering};
    use laplace_probe_sdk::{
        TrackedParkingLotRwLock, TrackedParkingLotRwLockReadGuard,
        TrackedParkingLotRwLockWriteGuard,
    };

    static SHARD_COUNTER: AtomicUsize = AtomicUsize::new(0);

    pub struct RwLock<T> {
        inner: TrackedParkingLotRwLock<T>,
    }

    impl<T> RwLock<T> {
        pub fn new(value: T) -> Self {
            let id = SHARD_COUNTER.fetch_add(1, Ordering::Relaxed);
            let name: &'static str = Box::leak(format!("dashmap_shard_{}", id).into_boxed_str());
            Self {
                inner: TrackedParkingLotRwLock::new(value, name),
            }
        }

        pub fn read(&self) -> RwLockReadGuard<'_, T> {
            RwLockReadGuard(self.inner.read())
        }

        pub fn write(&self) -> RwLockWriteGuard<'_, T> {
            RwLockWriteGuard {
                lock: self,
                guard: self.inner.write(),
            }
        }

        pub fn try_read(&self) -> Option<RwLockReadGuard<'_, T>> {
            self.inner.try_read().map(RwLockReadGuard)
        }

        pub fn try_write(&self) -> Option<RwLockWriteGuard<'_, T>> {
            self.inner
                .try_write()
                .map(|guard| RwLockWriteGuard { lock: self, guard })
        }

        pub fn data_ptr(&self) -> *mut T {
            self.inner.data_ptr()
        }
    }

    pub struct RwLockReadGuard<'a, T>(pub(crate) TrackedParkingLotRwLockReadGuard<'a, T>);

    pub struct RwLockWriteGuard<'a, T> {
        lock: &'a RwLock<T>,
        guard: TrackedParkingLotRwLockWriteGuard<'a, T>,
    }

    impl<'a, T> RwLockWriteGuard<'a, T> {
        // SAFETY: downgrade is called from unsafe context in DashMap.
        // We drop the write guard and acquire a new read guard on the same lock.
        // This is safe because we're converting exclusive to shared access.
        pub unsafe fn downgrade(self) -> RwLockReadGuard<'a, T> {
            let lock = self.lock;
            drop(self);
            RwLockReadGuard(lock.inner.read())
        }
    }

    impl<'a, T> std::ops::Deref for RwLockReadGuard<'a, T> {
        type Target = T;
        fn deref(&self) -> &T {
            &*self.0
        }
    }

    impl<'a, T> std::ops::Deref for RwLockWriteGuard<'a, T> {
        type Target = T;
        fn deref(&self) -> &T {
            &*self.guard
        }
    }

    impl<'a, T> std::ops::DerefMut for RwLockWriteGuard<'a, T> {
        fn deref_mut(&mut self) -> &mut T {
            &mut *self.guard
        }
    }
}

#[cfg(feature = "laplace")]
pub use laplace_lock::{RwLock, RwLockReadGuard, RwLockWriteGuard};

#[cfg(not(feature = "laplace"))]
pub type RwLock<T> = lock_api::RwLock<RawRwLock, T>;
#[cfg(not(feature = "laplace"))]
pub type RwLockReadGuard<'a, T> = lock_api::RwLockReadGuard<'a, RawRwLock, T>;
#[cfg(not(feature = "laplace"))]
pub type RwLockWriteGuard<'a, T> = lock_api::RwLockWriteGuard<'a, RawRwLock, T>;

// Only compile RawRwLock when not using laplace feature
#[cfg(not(feature = "laplace"))]
const READERS_PARKED: usize = 0b0001;
#[cfg(not(feature = "laplace"))]
const WRITERS_PARKED: usize = 0b0010;
#[cfg(not(feature = "laplace"))]
const ONE_READER: usize = 0b0100;
#[cfg(not(feature = "laplace"))]
const ONE_WRITER: usize = !(READERS_PARKED | WRITERS_PARKED);

#[cfg(not(feature = "laplace"))]
pub struct RawRwLock {
    state: AtomicUsize,
}

#[cfg(not(feature = "laplace"))]
unsafe impl lock_api::RawRwLock for RawRwLock {
    #[allow(clippy::declare_interior_mutable_const)]
    const INIT: Self = Self {
        state: AtomicUsize::new(0),
    };

    type GuardMarker = lock_api::GuardNoSend;

    #[inline]
    fn try_lock_exclusive(&self) -> bool {
        self.state
            .compare_exchange(0, ONE_WRITER, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    #[inline]
    fn lock_exclusive(&self) {
        if self
            .state
            .compare_exchange_weak(0, ONE_WRITER, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            self.lock_exclusive_slow();
        }
    }

    #[inline]
    unsafe fn unlock_exclusive(&self) {
        if self
            .state
            .compare_exchange(ONE_WRITER, 0, Ordering::Release, Ordering::Relaxed)
            .is_err()
        {
            self.unlock_exclusive_slow();
        }
    }

    #[inline]
    fn try_lock_shared(&self) -> bool {
        self.try_lock_shared_fast() || self.try_lock_shared_slow()
    }

    #[inline]
    fn lock_shared(&self) {
        if !self.try_lock_shared_fast() {
            self.lock_shared_slow();
        }
    }

    #[inline]
    unsafe fn unlock_shared(&self) {
        let state = self.state.fetch_sub(ONE_READER, Ordering::Release);

        if state == (ONE_READER | WRITERS_PARKED) {
            self.unlock_shared_slow();
        }
    }
}

#[cfg(not(feature = "laplace"))]
unsafe impl lock_api::RawRwLockDowngrade for RawRwLock {
    #[inline]
    unsafe fn downgrade(&self) {
        let state = self
            .state
            .fetch_and(ONE_READER | WRITERS_PARKED, Ordering::Release);
        if state & READERS_PARKED != 0 {
            parking_lot_core::unpark_all((self as *const _ as usize) + 1, UnparkToken(0));
        }
    }
}

#[cfg(not(feature = "laplace"))]
impl RawRwLock {
    #[cold]
    fn lock_exclusive_slow(&self) {
        let mut acquire_with = 0;
        loop {
            let mut spin = SpinWait::new();
            let mut state = self.state.load(Ordering::Relaxed);

            loop {
                while state & ONE_WRITER == 0 {
                    match self.state.compare_exchange_weak(
                        state,
                        state | ONE_WRITER | acquire_with,
                        Ordering::Acquire,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => return,
                        Err(e) => state = e,
                    }
                }

                if state & WRITERS_PARKED == 0 {
                    if spin.spin() {
                        state = self.state.load(Ordering::Relaxed);
                        continue;
                    }

                    if let Err(e) = self.state.compare_exchange_weak(
                        state,
                        state | WRITERS_PARKED,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    ) {
                        state = e;
                        continue;
                    }
                }

                let _ = unsafe {
                    parking_lot_core::park(
                        self as *const _ as usize,
                        || {
                            let state = self.state.load(Ordering::Relaxed);
                            (state & ONE_WRITER != 0) && (state & WRITERS_PARKED != 0)
                        },
                        || {},
                        |_, _| {},
                        ParkToken(0),
                        None,
                    )
                };

                acquire_with = WRITERS_PARKED;
                break;
            }
        }
    }

    #[cold]
    fn unlock_exclusive_slow(&self) {
        let state = self.state.load(Ordering::Relaxed);
        assert_eq!(state & ONE_WRITER, ONE_WRITER);

        let mut parked = state & (READERS_PARKED | WRITERS_PARKED);
        assert_ne!(parked, 0);

        if parked != (READERS_PARKED | WRITERS_PARKED) {
            if let Err(new_state) =
                self.state
                    .compare_exchange(state, 0, Ordering::Release, Ordering::Relaxed)
            {
                assert_eq!(new_state, ONE_WRITER | READERS_PARKED | WRITERS_PARKED);
                parked = READERS_PARKED | WRITERS_PARKED;
            }
        }

        if parked == (READERS_PARKED | WRITERS_PARKED) {
            self.state.store(WRITERS_PARKED, Ordering::Release);
            parked = READERS_PARKED;
        }

        if parked == READERS_PARKED {
            return unsafe {
                parking_lot_core::unpark_all((self as *const _ as usize) + 1, UnparkToken(0));
            };
        }

        assert_eq!(parked, WRITERS_PARKED);
        unsafe {
            parking_lot_core::unpark_one(self as *const _ as usize, |_| UnparkToken(0));
        }
    }

    #[inline(always)]
    fn try_lock_shared_fast(&self) -> bool {
        let state = self.state.load(Ordering::Relaxed);

        if let Some(new_state) = state.checked_add(ONE_READER) {
            if new_state & ONE_WRITER != ONE_WRITER {
                return self
                    .state
                    .compare_exchange_weak(state, new_state, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok();
            }
        }

        false
    }

    #[cold]
    fn try_lock_shared_slow(&self) -> bool {
        let mut state = self.state.load(Ordering::Relaxed);

        while let Some(new_state) = state.checked_add(ONE_READER) {
            if new_state & ONE_WRITER == ONE_WRITER {
                break;
            }

            match self.state.compare_exchange_weak(
                state,
                new_state,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(e) => state = e,
            }
        }

        false
    }

    #[cold]
    fn lock_shared_slow(&self) {
        loop {
            let mut spin = SpinWait::new();
            let mut state = self.state.load(Ordering::Relaxed);

            loop {
                let mut backoff = SpinWait::new();
                while let Some(new_state) = state.checked_add(ONE_READER) {
                    assert_ne!(
                        new_state & ONE_WRITER,
                        ONE_WRITER,
                        "reader count overflowed",
                    );

                    if self
                        .state
                        .compare_exchange_weak(
                            state,
                            new_state,
                            Ordering::Acquire,
                            Ordering::Relaxed,
                        )
                        .is_ok()
                    {
                        return;
                    }

                    backoff.spin_no_yield();
                    state = self.state.load(Ordering::Relaxed);
                }

                if state & READERS_PARKED == 0 {
                    if spin.spin() {
                        state = self.state.load(Ordering::Relaxed);
                        continue;
                    }

                    if let Err(e) = self.state.compare_exchange_weak(
                        state,
                        state | READERS_PARKED,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    ) {
                        state = e;
                        continue;
                    }
                }

                let _ = unsafe {
                    parking_lot_core::park(
                        (self as *const _ as usize) + 1,
                        || {
                            let state = self.state.load(Ordering::Relaxed);
                            (state & ONE_WRITER == ONE_WRITER) && (state & READERS_PARKED != 0)
                        },
                        || {},
                        |_, _| {},
                        ParkToken(0),
                        None,
                    )
                };

                break;
            }
        }
    }

    #[cold]
    fn unlock_shared_slow(&self) {
        if self
            .state
            .compare_exchange(WRITERS_PARKED, 0, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            unsafe {
                parking_lot_core::unpark_one(self as *const _ as usize, |_| UnparkToken(0));
            }
        }
    }
}
