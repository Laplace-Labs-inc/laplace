// SPDX-License-Identifier: Apache-2.0

#[derive(Clone, Copy)]
pub struct ThreadId;

#[derive(Clone, Copy)]
pub struct ResourceId;

#[derive(Clone, Copy)]
pub enum Operation {
    Request,
}

pub mod registry {
    use super::{Operation, ResourceId, ThreadId};

    #[derive(Clone, Copy)]
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
        pub pc_labels: &'static [(usize, usize, &'static str)],
    }
}

inventory::collect!(registry::HarnessConfig);

#[laplace_macro::axiom_harness(
    name = "public_contract",
    threads = 1,
    resources = 1,
    desc = "compile contract",
    expected = "clean",
    resource_names = ["r0"],
    thread_names = ["t0"]
)]
pub fn public_contract(_thread: ThreadId, _pc: usize) -> Option<(Operation, ResourceId)> {
    Some((Operation::Request, ResourceId))
}

fn main() {}
