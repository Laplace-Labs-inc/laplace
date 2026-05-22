// SPDX-License-Identifier: Apache-2.0
#![cfg(kani)]

//! Kani proofs for RAG semaphore invariants.

use super::rag::RagTracker;
use laplace_interfaces::domain::resource::{
    RequestResult, ResourceCapacity, ResourceError, ResourceId, ResourceTracker, ThreadId,
};

#[kani::proof]
fn proof_rag_semaphore_capacity_invariant() {
    let mut tracker = RagTracker::new_with_capacities(3, &[ResourceCapacity::new(2)]);

    assert_eq!(
        tracker.request(ThreadId(0), ResourceId(0)).unwrap(),
        RequestResult::Acquired
    );
    assert_eq!(
        tracker.request(ThreadId(1), ResourceId(0)).unwrap(),
        RequestResult::Acquired
    );
    assert_eq!(
        tracker.request(ThreadId(2), ResourceId(0)).unwrap(),
        RequestResult::Blocked
    );
}

#[kani::proof]
fn proof_rag_mutex_exclusive() {
    let mut tracker = RagTracker::new(2, 1);

    assert_eq!(
        tracker.request(ThreadId(0), ResourceId(0)).unwrap(),
        RequestResult::Acquired
    );

    match tracker.request(ThreadId(0), ResourceId(0)) {
        Err(ResourceError::AlreadyOwned { .. }) => {}
        _ => kani::assert(false, "mutex resource must reject duplicate holder"),
    }
}
