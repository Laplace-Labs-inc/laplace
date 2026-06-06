// SPDX-License-Identifier: Apache-2.0
//! TrackedAtomic* — `std::sync::atomic` 래퍼들.
//!
//! load → AtomicLoad, store → AtomicStore, CAS/fetch_* → AtomicRmw 이벤트 전송.
//!
//! [GHOST CONSTRAINT]: Phase 1에서는 Ordering 파라미터를 받되
//! 내부적으로 SeqCst를 강제한다. SimulatedMemory 브리지는 Phase 2.

#[cfg(laplace_private_verification)]
use crate::session::{current_thread_id, emit};

macro_rules! emit_probe_event {
    ($event:expr) => {
        #[cfg(laplace_private_verification)]
        {
            emit($event);
        }
    };
}
#[cfg(laplace_private_verification)]
use laplace_probe::ProbeEvent;
use std::sync::atomic::Ordering;

/// Helper trait for defining atomic type value representation.
/// Used by #[laplace_tracked] macro to determine default values for atomic types.
pub trait AtomicInner {
    /// The value type stored in this atomic.
    type Value;
}

impl AtomicInner for std::sync::atomic::AtomicBool {
    type Value = bool;
}

impl AtomicInner for std::sync::atomic::AtomicU32 {
    type Value = u32;
}

impl AtomicInner for std::sync::atomic::AtomicU64 {
    type Value = u64;
}

impl AtomicInner for std::sync::atomic::AtomicUsize {
    type Value = usize;
}

/// Macro to define TrackedAtomic* wrappers.
macro_rules! tracked_atomic {
    ($name:ident, $inner:ty) => {
        /// Tracked wrapper around a standard atomic type.
        #[cfg_attr(not(laplace_private_verification), allow(dead_code))]
        pub struct $name {
            inner: $inner,
            resource_name: &'static str,
        }

        impl $name {
            /// Creates a new tracked atomic with the given initial value.
            pub fn new(value: <$inner as AtomicInner>::Value, resource_name: &'static str) -> Self {
                Self {
                    inner: <$inner>::new(value),
                    resource_name,
                }
            }

            /// Loads the current value. Internally uses SeqCst ordering.
            pub fn load(&self, _ordering: Ordering) -> <$inner as AtomicInner>::Value {
                let val = self.inner.load(Ordering::SeqCst);
                emit_probe_event!(ProbeEvent::AtomicLoad {
                    thread_id: current_thread_id(),
                    resource: self.resource_name.to_string(),
                });
                val
            }

            /// Stores a new value. Internally uses SeqCst ordering.
            pub fn store(&self, value: <$inner as AtomicInner>::Value, _ordering: Ordering) {
                self.inner.store(value, Ordering::SeqCst);
                emit_probe_event!(ProbeEvent::AtomicStore {
                    thread_id: current_thread_id(),
                    resource: self.resource_name.to_string(),
                });
            }

            /// Atomically compares and exchanges the value. Internally uses SeqCst ordering.
            pub fn compare_exchange(
                &self,
                current: <$inner as AtomicInner>::Value,
                new: <$inner as AtomicInner>::Value,
                _success: Ordering,
                _failure: Ordering,
            ) -> Result<<$inner as AtomicInner>::Value, <$inner as AtomicInner>::Value> {
                let result =
                    self.inner
                        .compare_exchange(current, new, Ordering::SeqCst, Ordering::SeqCst);
                emit_probe_event!(ProbeEvent::AtomicRmw {
                    thread_id: current_thread_id(),
                    resource: self.resource_name.to_string(),
                });
                result
            }
        }
    };
}

/// Macro to define numeric TrackedAtomic* wrappers (with fetch_add/fetch_sub).
macro_rules! tracked_atomic_numeric {
    ($name:ident, $inner:ty) => {
        tracked_atomic!($name, $inner);

        impl $name {
            /// Atomically adds to the value. Internally uses SeqCst ordering.
            pub fn fetch_add(
                &self,
                val: <$inner as AtomicInner>::Value,
                _ordering: Ordering,
            ) -> <$inner as AtomicInner>::Value {
                let prev = self.inner.fetch_add(val, Ordering::SeqCst);
                emit_probe_event!(ProbeEvent::AtomicRmw {
                    thread_id: current_thread_id(),
                    resource: self.resource_name.to_string(),
                });
                prev
            }

            /// Atomically subtracts from the value. Internally uses SeqCst ordering.
            pub fn fetch_sub(
                &self,
                val: <$inner as AtomicInner>::Value,
                _ordering: Ordering,
            ) -> <$inner as AtomicInner>::Value {
                let prev = self.inner.fetch_sub(val, Ordering::SeqCst);
                emit_probe_event!(ProbeEvent::AtomicRmw {
                    thread_id: current_thread_id(),
                    resource: self.resource_name.to_string(),
                });
                prev
            }
        }
    };
}

tracked_atomic!(TrackedAtomicBool, std::sync::atomic::AtomicBool);
tracked_atomic_numeric!(TrackedAtomicU32, std::sync::atomic::AtomicU32);
tracked_atomic_numeric!(TrackedAtomicU64, std::sync::atomic::AtomicU64);
tracked_atomic_numeric!(TrackedAtomicUsize, std::sync::atomic::AtomicUsize);
