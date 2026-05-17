// SPDX-License-Identifier: Apache-2.0
//! Chaos injection infrastructure for Distributed Axiom network simulation.
//!
//! Provides [`ChaosInterceptor`], a [`PacketInterceptor`](laplace_interfaces::domain::transport::pluggable::PacketInterceptor)
//! implementation that drops or delays packets according to a [`ChaosSchedule`](laplace_interfaces::domain::kraken::types::ChaosSchedule).

pub mod interceptor;

pub use interceptor::ChaosInterceptor;
