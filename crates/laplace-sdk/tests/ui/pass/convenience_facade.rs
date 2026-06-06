// SPDX-License-Identifier: Apache-2.0

fn main() {
    let _counter = laplace_sdk::mutex!(1usize, "counter");
    let _state = laplace_sdk::rwlock!(String::from("ready"), "state");
}
