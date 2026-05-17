// SPDX-License-Identifier: Apache-2.0
//! Laplace BYOC Audit — unified external concurrency verification crate.
//!
//! This crate combines:
//! - deadpool 0.10 surgical audit modules
//! - BYOC hunt harness modules (bytes, futures-util, mio, parking_lot)

#![forbid(unsafe_code)]
#![allow(missing_docs)]
#![allow(unused_results)]
#![allow(deprecated)]

pub mod axiom_compat;
pub mod managed;

pub mod bytes_hunt;
pub mod futures_util_hunt;
pub mod mio_hunt;
pub mod parking_lot_hunt;

pub use deadpool_runtime::{Runtime, SpawnBlockingError};

/// Pool status snapshot — mirrors `deadpool::Status`.
#[derive(Clone, Copy, Debug)]
pub struct Status {
    pub max_size: usize,
    pub size: usize,
    pub available: usize,
    pub waiting: usize,
}
