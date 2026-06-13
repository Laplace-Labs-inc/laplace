// SPDX-License-Identifier: Apache-2.0
//! Single-source AB-BA controlled-reexecution program body.
//!
//! This is the **one** definition of the deadpool-hunt AB-BA lock-order program.
//! Both paths execute THIS body — there is no second hand-authored AB-BA program:
//!   * the public passive toy scanner (`TrackedStdMutex` + `std::thread::spawn`), and
//!   * the private engine route (`laplace_sync::Mutex` + `LiveEnv::spawn`).
//!
//! The lock surface ([`ModelLock`]) and the spawn surface are injected so the
//! exact same source drives both. The file is pure `std` + the injected traits;
//! it names no concrete mutex type, so neither path can fork the program shape.
//!
//! Scope honesty: this is a **developer seam**, not the unmodified-`std` funnel.
//! A user still writes the body against [`ModelLock`]/the spawn facade rather
//! than raw `std::sync::Mutex` + `std::thread::spawn`. Routing genuinely
//! unmodified user source requires either a source-rewriting proc-macro or a
//! public hook-capable shadow primitive (see DEBT-BYOC-1b ledger).

use std::sync::Arc;

/// Injected model lock.
///
/// `hold` acquires the resource, runs `inner` while held, then releases —
/// expressing nested AB-BA ordering without crossing a guard across the trait
/// boundary (which keeps the trait object-safe).
pub trait ModelLock: Send + Sync + 'static {
    /// Acquires the resource, runs `inner` while held, then releases.
    fn hold(&self, inner: &mut dyn FnMut());
}

/// Distinct resources the program declares, in declaration order.
///
/// The coverage denominator is derived from this list, so the numerator
/// (resources the engine actually drove) and the denominator measure the
/// **same** program.
pub const AB_BA_RESOURCES: [&str; 2] = ["pool_state", "conn_meta"];

/// The single AB-BA program body.
///
/// `make_lock` builds a shared model lock for a named resource; `spawn` runs one
/// model thread. Thread 0 takes A then B; thread 1 takes B then A (the AB-BA
/// lock-order inversion).
pub fn deadpool_ab_ba_program(
    make_lock: &dyn Fn(&'static str) -> Arc<dyn ModelLock>,
    spawn: &dyn Fn(Box<dyn FnOnce() + Send + 'static>),
) {
    let a = make_lock(AB_BA_RESOURCES[0]);
    let b = make_lock(AB_BA_RESOURCES[1]);

    {
        let a = a.clone();
        let b = b.clone();
        spawn(Box::new(move || {
            a.hold(&mut || b.hold(&mut || {}));
        }));
    }
    {
        let a = a.clone();
        let b = b.clone();
        spawn(Box::new(move || {
            b.hold(&mut || a.hold(&mut || {}));
        }));
    }
}
