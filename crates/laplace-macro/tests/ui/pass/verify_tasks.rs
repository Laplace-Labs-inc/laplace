// SPDX-License-Identifier: Apache-2.0
#![allow(dead_code)]

#[laplace_macro::laplace_verify(tasks)]
fn task_composition(tasks: &mut laplace_rt::TaskSet) {
    let _handle = tasks.spawn(async {});
}

fn main() {}
