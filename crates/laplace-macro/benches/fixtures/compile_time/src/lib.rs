// SPDX-License-Identifier: Apache-2.0
#![allow(dead_code)]

use laplace_macro::laplace_meta;

pub type ThreadId = usize;

#[derive(Clone, Copy)]
pub enum Operation {
    Request,
    Release,
}

#[derive(Clone, Copy)]
pub struct ResourceId(pub usize);

pub mod registry {
    use super::{Operation, ResourceId, ThreadId};

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
}

inventory::collect!(registry::HarnessConfig);

#[laplace_meta(bench = "compile_time", seed = 42)]
pub fn passthrough_attribute_fixture() -> u64 {
    42
}

#[laplace_macro::axiom_harness(
    name = "macro_compile_seed_42",
    threads = 2,
    resources = 1,
    desc = "deterministic compile-time fixture",
    expected = "clean",
    resource_names = ["resource_seed_42"],
    thread_names = ["thread_0", "thread_1"]
)]
pub fn harness_fixture(_thread: ThreadId, pc: usize) -> Option<(Operation, ResourceId)> {
    match pc {
        0 => Some((Operation::Request, ResourceId(0))),
        1 => Some((Operation::Release, ResourceId(0))),
        _ => None,
    }
}

pub fn consume_fixture_shape() -> u64 {
    let config = inventory::iter::<registry::HarnessConfig>
        .into_iter()
        .find(|config| config.name == "macro_compile_seed_42");

    match config {
        Some(config) => config.num_threads as u64 + config.num_resources as u64,
        None => passthrough_attribute_fixture(),
    }
}
