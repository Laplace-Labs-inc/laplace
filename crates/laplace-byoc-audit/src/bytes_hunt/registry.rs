// SPDX-License-Identifier: Apache-2.0
//! Local harness registry for laplace-bytes-hunt.

use laplace_core::domain::resource::{ResourceId, ThreadId};
use laplace_dpor::Operation;

pub struct HarnessConfig {
    pub name: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub num_threads: usize,
    pub num_resources: usize,
    pub op_provider: fn(ThreadId, usize) -> Option<(Operation, ResourceId)>,
    pub expected: &'static str,
    pub resource_names: &'static [&'static str],
    pub thread_names: &'static [&'static str],
    pub pc_labels: &'static [&'static str],
}

inventory::collect!(HarnessConfig);
