//! A full-featured connection pool, designed for asynchronous connections
//! (using tokio). Originally based on [r2d2](https://github.com/sfackler/r2d2).
//!
//! Opening a new database connection every time one is needed is both
//! inefficient and can lead to resource exhaustion under high traffic
//! conditions. A connection pool maintains a set of open connections to a
//! database, handing them out for repeated use.
//!
//! bb8 is agnostic to the connection type it is managing. Implementors of the
//! `ManageConnection` trait provide the database-specific logic to create and
//! check the health of connections.
//!
//! # Example
//!
//! Using an imaginary "foodb" database.
//!
//! ```ignore
//! #[tokio::main]
//! async fn main() {
//!     let manager = bb8_foodb::FooConnectionManager::new("localhost:1234");
//!     let pool = bb8::Pool::builder().build(manager).await.unwrap();
//!
//!     for _ in 0..20 {
//!         let pool = pool.clone();
//!         tokio::spawn(async move {
//!             let conn = pool.get().await.unwrap();
//!             // use the connection
//!             // it will be returned to the pool when it falls out of scope.
//!         });
//!     }
//! }
//! ```
#![allow(clippy::needless_doctest_main)]
#![deny(missing_docs, missing_debug_implementations)]

mod api;
pub use api::{
    Builder, CustomizeConnection, ErrorSink, ManageConnection, NopErrorSink, Pool,
    PooledConnection, QueueStrategy, RunError, State,
};

mod inner;
mod internals;
mod lock {
    // ── [LAPLACE PATCH] TrackedStdMutex for Ki-DPOR verification ─────────────
    #[cfg(feature = "laplace")]
    pub(crate) struct Mutex<T>(laplace_probe_sdk::TrackedStdMutex<T>);

    #[cfg(feature = "laplace")]
    impl<T> Mutex<T> {
        pub(crate) fn new(val: T) -> Self {
            Self(laplace_probe_sdk::TrackedStdMutex::new(
                val,
                "bb8_pool_internals",
            ))
        }

        pub(crate) fn lock(&self) -> laplace_probe_sdk::TrackedStdGuard<'_, T> {
            self.0.lock()
        }
    }

    // ── parking_lot (default, production) ────────────────────────────────────
    #[cfg(all(feature = "parking_lot", not(feature = "laplace")))]
    use parking_lot::Mutex as MutexImpl;
    #[cfg(all(feature = "parking_lot", not(feature = "laplace")))]
    use parking_lot::MutexGuard;

    // ── std::sync fallback ───────────────────────────────────────────────────
    #[cfg(not(any(feature = "parking_lot", feature = "laplace")))]
    use std::sync::Mutex as MutexImpl;
    #[cfg(not(any(feature = "parking_lot", feature = "laplace")))]
    use std::sync::MutexGuard;

    // ── non-laplace wrapper ──────────────────────────────────────────────────
    #[cfg(not(feature = "laplace"))]
    pub(crate) struct Mutex<T>(MutexImpl<T>);

    #[cfg(not(feature = "laplace"))]
    impl<T> Mutex<T> {
        pub(crate) fn new(val: T) -> Self {
            Self(MutexImpl::new(val))
        }

        pub(crate) fn lock(&self) -> MutexGuard<'_, T> {
            #[cfg(feature = "parking_lot")]
            {
                self.0.lock()
            }
            #[cfg(not(feature = "parking_lot"))]
            {
                self.0.lock().unwrap()
            }
        }
    }
}
