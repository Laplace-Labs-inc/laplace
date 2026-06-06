// SPDX-License-Identifier: Apache-2.0

fn main() {
    let _support_lock =
        laplace_probe_sdk::__macro_support::TrackedMutex::named(0usize, "support_counter");
    let _counter = laplace_macro::mutex!(1usize, "counter");
    let _state = laplace_macro::rwlock!(String::from("ready"), "state");
}
