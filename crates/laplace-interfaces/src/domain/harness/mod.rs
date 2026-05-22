// SPDX-License-Identifier: Apache-2.0
//! Verification harness specification contracts.

mod builder;
mod spec;

pub use builder::{DeadlockProbe, GateBuilder, ResourceBuilder, ThreadBuilder};
pub use spec::{
    GateId, GateSpec, HarnessSpec, ReleasePolicy, ResourceId, ResourceSpec, ThreadAction,
    ThreadSpec,
};
