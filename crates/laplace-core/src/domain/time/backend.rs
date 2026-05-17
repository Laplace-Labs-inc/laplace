// SPDX-License-Identifier: Apache-2.0
//! Clock backend trait — re-exported from `laplace-interfaces`
//!
//! The canonical `ClockBackend` trait lives in `laplace_interfaces::domain::time::traits`.
//! This file re-exports it so that code within `laplace-core` can continue to use
//! `crate::domain::time::ClockBackend`.

pub use laplace_interfaces::domain::time::traits::ClockBackend;
