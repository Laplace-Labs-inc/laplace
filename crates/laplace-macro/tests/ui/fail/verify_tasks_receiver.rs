// SPDX-License-Identifier: Apache-2.0

struct Composition;

impl Composition {
    #[laplace_macro::laplace_verify(tasks)]
    fn composition_with_receiver(&mut self) {}
}

fn main() {}
