// SPDX-License-Identifier: Apache-2.0
pub mod benchmark;
pub mod bytes;
pub mod core_resource;
pub mod credit_meter;
pub mod entropy;
pub mod futures_util;
pub mod journal;
pub mod liveness;
pub mod memory;
pub mod mio;
pub mod parking_lot;
pub mod pool;
pub mod resource_abba;
pub mod scheduler;
pub mod telemetry;
pub mod template;
pub mod time;
// Coverage-boundary: the sole harness here (`tracing_causality_acyclicity`)
// models a self-deadlock the frozen engine does not flag. Off by default.
#[cfg(feature = "scenarios-coverage-boundary")]
pub mod tracing;
