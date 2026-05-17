// SPDX-License-Identifier: Apache-2.0
//! Memory Backend Trait Definitions — re-exported from `laplace-interfaces`
//!
//! All canonical trait definitions live in `laplace_interfaces::domain::memory::traits`.
//! This file re-exports them so that code within `laplace-core` can continue to use the
//! short path `crate::domain::memory::{MemoryBackend, ConfigurableBackend}`.

pub use laplace_interfaces::domain::memory::traits::{ConfigurableBackend, MemoryBackend};
