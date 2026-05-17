// SPDX-License-Identifier: Apache-2.0
//! Tracer backend trait — re-exported from `laplace-interfaces`
//!
//! The canonical `TracerBackend` trait and `TracingError` type live in
//! `laplace_interfaces::domain::tracing::traits`. This file re-exports them so
//! that code within `laplace-core` can continue to use
//! `crate::domain::tracing::{TracerBackend, TracingError}`.

pub use laplace_interfaces::domain::tracing::traits::{TracerBackend, TracingError};
