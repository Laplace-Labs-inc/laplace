// SPDX-License-Identifier: Apache-2.0

use laplace_sdk::prelude::*;
use std::sync::atomic::Ordering;

#[laplace_tracked]
struct Service {
    #[track(name = "jobs")]
    jobs: Mutex<Vec<String>>,

    #[track]
    version: AtomicUsize,
}

fn main() {
    let _support_lock =
        laplace_sdk::__macro_support::TrackedMutex::named(0usize, "support_counter");
    let service = Service::default();
    service.version.store(1, Ordering::SeqCst);
    assert_eq!(service.version.load(Ordering::SeqCst), 1);

    let _jobs = service.jobs;
}
