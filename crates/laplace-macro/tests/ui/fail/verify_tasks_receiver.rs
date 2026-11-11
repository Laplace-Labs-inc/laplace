// SPDX-License-Identifier: MIT

struct Composition;

impl Composition {
    #[laplace_macro::laplace_verify(tasks)]
    fn composition_with_receiver(&mut self) {}
}

fn main() {}
