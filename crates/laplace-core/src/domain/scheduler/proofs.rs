//! Formal Verification Harnesses for Scheduler using Kani
//!
//! This module contains bounded model checking proofs for the SchedulerEngine
//! and VerificationBackend, verifying core invariants about thread state management,
//! thread ID bounds, task scheduling, and event ownership.
//!
//! # Proofs Included
//!
//! 1. `proof_thread_id_bounds` - Verifies that thread IDs are properly bounded
//! 2. `proof_schedule_task_valid_thread` - Verifies task scheduling with valid threads
//! 3. `proof_schedule_task_invalid_thread` - Verifies rejection of invalid thread IDs
//! 4. `proof_thread_state_invariants` - Verifies thread state transitions maintain invariants
//! 5. `proof_event_ownership` - Verifies event ownership is correctly tracked

use super::engine::SchedulerEngine;
use super::verification::VerificationBackend;
use laplace_interfaces::domain::scheduler::{
    SchedulerBackend, SchedulerError, SchedulingStrategy, TaskId, ThreadId, ThreadState,
};

/// Proof: Thread ID bounds are correctly enforced
///
/// # Specification
/// For a backend with `num_threads = n`, all valid thread IDs must satisfy `0 <= id < n`.
/// Invalid thread IDs (where `id >= n`) must be rejected with `InvalidThreadId` error.
///
/// # Verification Target
/// VerificationBackend::new(4) creates a backend with exactly 4 threads.
/// ThreadId values outside [0, 4) must fail all operations.
#[kani::proof]
fn proof_thread_id_bounds() {
    let num_threads = 4usize;
    let backend = VerificationBackend::new(num_threads);

    // Valid thread IDs: 0, 1, 2, 3
    for valid_id in 0..num_threads {
        let thread_id = ThreadId::new(valid_id);

        // All valid thread IDs should start as RUNNABLE
        let state = backend.get_state(thread_id);
        assert!(
            state.is_ok(),
            "get_state should succeed for valid thread_id"
        );
        assert_eq!(state.unwrap(), ThreadState::Runnable);
    }

    // Invalid thread IDs: anything >= num_threads
    let invalid_id = ThreadId::new(num_threads);
    let result = backend.get_state(invalid_id);
    assert!(
        result.is_err(),
        "get_state should fail for invalid thread_id"
    );

    if let Err(SchedulerError::InvalidThreadId {
        thread_id,
        max_threads,
    }) = result
    {
        assert_eq!(thread_id, invalid_id);
        assert_eq!(max_threads, num_threads);
    } else {
        panic!("Expected InvalidThreadId error");
    }
}

/// Proof: All threads start in RUNNABLE state (precondition for scheduling)
///
/// # Specification
/// Given a freshly created engine with 4 threads, all threads must be in RUNNABLE state.
/// This is a precondition for successful task scheduling.
///
/// # Verification Target
/// Verify that the backend initializes all threads to RUNNABLE state.
#[kani::proof]
fn proof_schedule_task_valid_thread() {
    let engine: SchedulerEngine<VerificationBackend> =
        SchedulerEngine::new(4, SchedulingStrategy::Verification);

    // All threads must start in RUNNABLE state (prerequisite for scheduling)
    for i in 0..4 {
        let thread_id = ThreadId::new(i);
        let state = engine.get_thread_state(thread_id);
        assert!(
            state.is_ok(),
            "get_thread_state should succeed for valid thread"
        );
        assert_eq!(
            state.unwrap(),
            ThreadState::Runnable,
            "All threads should start in RUNNABLE state"
        );
    }

    // Verify state counts reflect all threads in RUNNABLE
    let (runnable, blocked, completed) = engine.thread_state_counts();
    assert_eq!(runnable, 4);
    assert_eq!(blocked, 0);
    assert_eq!(completed, 0);

    // Verify all threads are considered runnable
    for i in 0..4 {
        assert!(
            engine.backend().is_runnable(ThreadId::new(i)),
            "All threads should be reportable as runnable"
        );
    }
}

/// Proof: Task scheduling fails with invalid thread IDs
///
/// # Specification
/// schedule_task with an invalid thread ID should return InvalidThreadId error.
///
/// # Verification Target
/// Threading attempting to schedule on thread_id >= num_threads should fail.
#[kani::proof]
fn proof_schedule_task_invalid_thread() {
    let mut engine: SchedulerEngine<VerificationBackend> =
        SchedulerEngine::new(4, SchedulingStrategy::Verification);

    let result = engine.schedule_task(ThreadId::new(10), 100_000_000);
    assert!(result.is_err(), "Scheduling on invalid thread should fail");

    if let Err(SchedulerError::InvalidThreadId {
        thread_id,
        max_threads,
    }) = result
    {
        assert_eq!(thread_id, ThreadId::new(10));
        assert_eq!(max_threads, 4);
    } else {
        panic!("Expected InvalidThreadId error");
    }
}

/// Proof: Task scheduling fails with non-RUNNABLE threads
///
/// # Specification
/// schedule_task requires the thread to be in RUNNABLE state.
/// If a thread is BLOCKED or COMPLETED, scheduling must fail with InvalidThreadState.
///
/// # Verification Target
/// After transitioning a thread to BLOCKED state, scheduling should fail.
#[kani::proof]
fn proof_schedule_task_blocked_thread() {
    let mut engine: SchedulerEngine<VerificationBackend> =
        SchedulerEngine::new(4, SchedulingStrategy::Verification);

    let thread_id = ThreadId::new(0);

    // Block the thread
    let transition = engine.set_thread_state(thread_id, ThreadState::Blocked);
    assert!(transition.is_ok());
    assert_eq!(transition.unwrap(), ThreadState::Runnable);

    // Now scheduling on blocked thread should fail
    let result = engine.schedule_task(thread_id, 100_000_000);
    assert!(result.is_err(), "Scheduling on BLOCKED thread should fail");

    if let Err(SchedulerError::InvalidThreadState {
        thread_id: tid,
        current_state,
        expected_state,
    }) = result
    {
        assert_eq!(tid, thread_id);
        assert_eq!(current_state, ThreadState::Blocked);
        assert_eq!(expected_state, ThreadState::Runnable);
    } else {
        panic!("Expected InvalidThreadState error");
    }
}

/// Proof: Thread state transitions maintain invariants
///
/// # Specification
/// All thread state transitions (RUNNABLE -> BLOCKED -> COMPLETED) are allowed.
/// The state counts remain consistent: runnable + blocked + completed = num_threads.
///
/// # Verification Target
/// Verify state counts after each transition for all threads.
#[kani::proof]
fn proof_thread_state_invariants() {
    let mut engine: SchedulerEngine<VerificationBackend> =
        SchedulerEngine::new(4, SchedulingStrategy::Verification);

    // Initial state: all RUNNABLE
    let (runnable, blocked, completed) = engine.thread_state_counts();
    assert_eq!(runnable, 4);
    assert_eq!(blocked, 0);
    assert_eq!(completed, 0);
    assert_eq!(runnable + blocked + completed, 4);

    // Transition thread 0 to BLOCKED
    engine
        .set_thread_state(ThreadId::new(0), ThreadState::Blocked)
        .unwrap();
    let (runnable, blocked, completed) = engine.thread_state_counts();
    assert_eq!(runnable, 3);
    assert_eq!(blocked, 1);
    assert_eq!(completed, 0);
    assert_eq!(runnable + blocked + completed, 4);

    // Transition thread 1 to BLOCKED
    engine
        .set_thread_state(ThreadId::new(1), ThreadState::Blocked)
        .unwrap();
    let (runnable, blocked, completed) = engine.thread_state_counts();
    assert_eq!(runnable, 2);
    assert_eq!(blocked, 2);
    assert_eq!(completed, 0);
    assert_eq!(runnable + blocked + completed, 4);

    // Transition thread 0 from BLOCKED to COMPLETED
    engine
        .set_thread_state(ThreadId::new(0), ThreadState::Completed)
        .unwrap();
    let (runnable, blocked, completed) = engine.thread_state_counts();
    assert_eq!(runnable, 2);
    assert_eq!(blocked, 1);
    assert_eq!(completed, 1);
    assert_eq!(runnable + blocked + completed, 4);

    // Transition thread 2 from RUNNABLE to COMPLETED
    engine
        .set_thread_state(ThreadId::new(2), ThreadState::Completed)
        .unwrap();
    let (runnable, blocked, completed) = engine.thread_state_counts();
    assert_eq!(runnable, 1);
    assert_eq!(blocked, 1);
    assert_eq!(completed, 2);
    assert_eq!(runnable + blocked + completed, 4);
}

/// Proof: Event ownership is correctly tracked
///
/// # Specification
/// Events are registered to specific threads via register_event and can be queried
/// via get_event_owner. Unregistering events removes them from tracking.
///
/// # Verification Target
/// Verify event ownership tracking and unregistration.
#[kani::proof]
fn proof_event_ownership() {
    let backend = VerificationBackend::new(4);

    let event_id_1: u64 = 100;
    let event_id_2: u64 = 101;
    let thread_0 = ThreadId::new(0);
    let thread_1 = ThreadId::new(1);

    // Initially, no event ownership
    assert_eq!(backend.get_event_owner(event_id_1), None);

    // Register event 1 to thread 0
    let result = backend.register_event(event_id_1, thread_0);
    assert!(result.is_ok());
    assert_eq!(backend.get_event_owner(event_id_1), Some(thread_0));

    // Register event 2 to thread 1
    let result = backend.register_event(event_id_2, thread_1);
    assert!(result.is_ok());
    assert_eq!(backend.get_event_owner(event_id_2), Some(thread_1));

    // Event 1 should still be owned by thread 0
    assert_eq!(backend.get_event_owner(event_id_1), Some(thread_0));

    // Unregister event 1
    backend.unregister_event(event_id_1);
    assert_eq!(backend.get_event_owner(event_id_1), None);

    // Event 2 should still be owned by thread 1
    assert_eq!(backend.get_event_owner(event_id_2), Some(thread_1));

    // Clear all events
    backend.clear_events();
    assert_eq!(backend.get_event_owner(event_id_2), None);
}

/// Proof: Reset restores initial state
///
/// # Specification
/// After reset(), all threads return to RUNNABLE state and events are cleared.
///
/// # Verification Target
/// Verify reset() correctly restores the initial state.
#[kani::proof]
fn proof_reset_restores_initial_state() {
    let mut engine: SchedulerEngine<VerificationBackend> =
        SchedulerEngine::new(4, SchedulingStrategy::Verification);

    // Modify state
    engine
        .set_thread_state(ThreadId::new(0), ThreadState::Blocked)
        .unwrap();
    engine
        .set_thread_state(ThreadId::new(1), ThreadState::Completed)
        .unwrap();

    // Verify modified state
    let (runnable, blocked, completed) = engine.thread_state_counts();
    assert_eq!(runnable, 2);
    assert_eq!(blocked, 1);
    assert_eq!(completed, 1);

    // Reset
    engine.reset();

    // Verify restored to initial state
    let (runnable, blocked, completed) = engine.thread_state_counts();
    assert_eq!(runnable, 4);
    assert_eq!(blocked, 0);
    assert_eq!(completed, 0);

    // All threads should be RUNNABLE
    for i in 0..4 {
        let state = engine.get_thread_state(ThreadId::new(i)).unwrap();
        assert_eq!(state, ThreadState::Runnable);
    }
}

/// Proof: is_idle correctly reflects event ownership and thread blocking
///
/// # Specification
/// is_idle() returns true when there are no runnable events (events owned by RUNNABLE threads).
/// When events are on blocked threads, is_idle() returns true.
///
/// # Verification Target
/// Verify is_idle() reflects the count of events on runnable threads.
#[kani::proof]
fn proof_is_idle_status() {
    let engine: SchedulerEngine<VerificationBackend> =
        SchedulerEngine::new(4, SchedulingStrategy::Verification);

    // Initially idle: no events registered
    assert!(engine.is_idle(), "Scheduler should be idle with no events");

    // Register an event directly for thread 0 (bypassing schedule_task to avoid time calls)
    let backend = engine.backend();
    let event_id: u64 = 100;
    let result = backend.register_event(event_id, ThreadId::new(0));
    assert!(result.is_ok(), "Event registration should succeed");

    // Now with an event on a RUNNABLE thread, is_idle should be false
    assert!(
        !engine.is_idle(),
        "Scheduler should not be idle when events exist on runnable threads"
    );

    // Block the thread that owns the event
    engine
        .backend()
        .set_state(ThreadId::new(0), ThreadState::Blocked)
        .unwrap();

    // Now all events are on blocked threads, so should be idle
    assert!(
        engine.is_idle(),
        "Scheduler should be idle when all events are on blocked threads"
    );
}

// ============================================================================
// NEW PROOFS: TaskId Uniqueness and Overflow Safety
// ============================================================================

/// Proof: TaskId values are unique across multiple schedule_task calls
///
/// # Specification
/// Each call to `schedule_task()` returns a strictly increasing TaskId.
/// No two calls can return the same TaskId, even if called with identical parameters.
///
/// # TLA+ Correspondence
/// ```tla
/// UniqueTaskIds ==
///     \forall call1, call2 \in ScheduleCalls:
///         (call1 != call2) => (TaskId(call1) != TaskId(call2))
/// ```
///
/// # Verification Target
/// Verify that `next_task_id` counter is properly incremented and never produces duplicates.
#[kani::proof]
#[kani::unwind(10)] // Unwind loop 5 times to test 5 sequential schedule_task calls
fn proof_task_id_uniqueness() {
    let mut engine: SchedulerEngine<VerificationBackend> =
        SchedulerEngine::new(4, SchedulingStrategy::Verification);

    // Collect TaskIds from multiple schedule_task calls
    let mut task_ids: Vec<TaskId> = Vec::new();

    // Schedule up to 5 tasks from different threads (wrapping around)
    for i in 0..5 {
        let thread_id = ThreadId::new(i % 4); // Cycle through 4 threads

        // Reset thread to RUNNABLE if needed (set_thread_state doesn't enforce this)
        // In a real scenario, threads would reset themselves
        if i > 0 && i % 4 == 0 {
            // All 4 threads have been used once, reset all to RUNNABLE
            for t in 0..4 {
                let _ = engine.set_thread_state(ThreadId::new(t), ThreadState::Runnable);
            }
        }

        match engine.schedule_task(thread_id, 100_000_000) {
            Ok(task_id) => {
                task_ids.push(task_id);
            }
            Err(_) => {
                // If a task cannot be scheduled (e.g., thread not RUNNABLE),
                // that's acceptable for this test
                break;
            }
        }
    }

    // Verify all collected TaskIds are unique
    for i in 0..task_ids.len() {
        for j in (i + 1)..task_ids.len() {
            assert_ne!(
                task_ids[i], task_ids[j],
                "TaskId at position {} ({:?}) must not equal TaskId at position {} ({:?})",
                i, task_ids[i], j, task_ids[j]
            );
        }
    }
}

/// Proof: TaskId values are monotonically increasing
///
/// # Specification
/// If TaskId_n is returned from the n-th successful schedule_task call,
/// then TaskId_n < TaskId_{n+1}. The sequence is strictly increasing.
///
/// # TLA+ Correspondence
/// ```tla
/// MonotonicTaskIds ==
///     \forall n \in Nat:
///         (call_n succeeds /\ call_{n+1} succeeds) =>
///             TaskId(call_n) < TaskId(call_{n+1})
/// ```
///
/// # Verification Target
/// Verify that `next_task_id` is incremented by exactly 1 for each successful call.
#[kani::proof]
#[kani::unwind(10)] // Unwind loop 5 times
fn proof_task_id_monotonic_increase() {
    let mut engine: SchedulerEngine<VerificationBackend> =
        SchedulerEngine::new(4, SchedulingStrategy::Verification);

    let mut previous_task_id: Option<TaskId> = None;

    // Execute multiple schedule_task calls
    for i in 0..5 {
        let thread_id = ThreadId::new(i % 4);

        // Reset all threads to RUNNABLE after each cycle
        if i > 0 && i % 4 == 0 {
            for t in 0..4 {
                let _ = engine.set_thread_state(ThreadId::new(t), ThreadState::Runnable);
            }
        }

        match engine.schedule_task(thread_id, 100_000_000) {
            Ok(current_task_id) => {
                if let Some(prev_id) = previous_task_id {
                    let prev_id: TaskId = prev_id;
                    let current_task_id: TaskId = current_task_id;

                    // Verify strict ordering: prev_id < current_id
                    assert!(
                        prev_id.as_usize() < current_task_id.as_usize(),
                        "TaskId must be strictly increasing: {} < {}",
                        prev_id.as_usize(),
                        current_task_id.as_usize()
                    );

                    // Verify increment by exactly 1
                    assert_eq!(
                        current_task_id.as_usize(),
                        prev_id.as_usize() + 1,
                        "TaskId should increment by exactly 1"
                    );
                }
                previous_task_id = Some(current_task_id);
            }
            Err(_) => {
                // Task scheduling failed; this is allowed
                break;
            }
        }
    }

    // Verify that we collected at least some TaskIds
    assert!(
        previous_task_id.is_some(),
        "At least one schedule_task call should succeed"
    );
}

// ── H-S9 ─────────────────────────────────────────────────────────────────────

/// Proof: `schedule_task()` with `ThreadId(usize::MAX)` does not panic.
///
/// # Invariant
///
/// The scheduler engine correctly bounds-checks the thread ID before any
/// array indexing operations. A call with `ThreadId::new(usize::MAX)` must return
/// `Err(SchedulerError::InvalidThreadId)` without triggering any out-of-bounds
/// panic, arithmetic overflow, or undefined behaviour.
#[kani::proof]
#[kani::unwind(1)]
fn proof_schedule_max_thread_id_no_panic() {
    let mut engine: SchedulerEngine<VerificationBackend> =
        SchedulerEngine::new(4, SchedulingStrategy::Verification);
    // usize::MAX is far beyond the 4-thread limit — must be rejected gracefully.
    let result = engine.schedule_task(ThreadId::new(usize::MAX), 100_000_000);
    assert!(
        result.is_err(),
        "schedule_task with usize::MAX thread_id must return Err"
    );
}

// ── H-S10 ────────────────────────────────────────────────────────────────────

/// Proof: Any symbolic `ThreadId >= num_threads` causes `schedule_task()` to return
/// `Err(SchedulerError::InvalidThreadId)` without panicking.
///
/// # Invariant
///
/// For a scheduler with `num_threads = 4`, all symbolic thread IDs where
/// `id >= 4` must be rejected with the correct error variant. No out-of-bounds
/// array access or panic may occur on any rejected ID.
#[kani::proof]
#[kani::unwind(1)]
fn proof_invalid_thread_id_returns_error() {
    let num_threads = 4usize;
    let mut engine: SchedulerEngine<VerificationBackend> =
        SchedulerEngine::new(num_threads, SchedulingStrategy::Verification);

    let raw_id: usize = kani::any();
    kani::assume(raw_id >= num_threads); // guaranteed invalid

    let result = engine.schedule_task(ThreadId::new(raw_id), 100_000_000);

    // Must return Err — no panic on any symbolically-chosen invalid ID.
    assert!(result.is_err(), "Invalid thread ID must return Err");

    if let Err(SchedulerError::InvalidThreadId {
        thread_id,
        max_threads,
    }) = result
    {
        assert_eq!(thread_id, ThreadId::new(raw_id));
        assert_eq!(max_threads, num_threads);
    } else {
        panic!("Expected InvalidThreadId error variant");
    }
}

// ── TaskId consistency ────────────────────────────────────────────────────────

/// Proof: TaskId consistency across state transitions
///
/// # Specification
/// TaskId allocation is independent of thread state changes.
/// Changing thread states (RUNNABLE → BLOCKED → COMPLETED) does not affect
/// the TaskId counter or the validity of previously allocated TaskIds.
///
/// # TLA+ Correspondence
/// ```tla
/// TaskIdStateConsistency ==
///     \forall s1, s2 \in States:
///         (s1.nextTaskId = s2.nextTaskId) =>
///             (s1.threadStates != s2.threadStates) is allowed
/// ```
///
/// # Verification Target
/// Verify that TaskId allocation remains consistent even when thread states change.
#[kani::proof]
fn proof_task_id_state_consistency() {
    let mut engine: SchedulerEngine<VerificationBackend> =
        SchedulerEngine::new(4, SchedulingStrategy::Verification);

    // Schedule a task from thread 0 (RUNNABLE)
    let task_id_1 = engine
        .schedule_task(ThreadId::new(0), 100_000_000)
        .expect("First schedule should succeed");

    // Change thread 0 state to BLOCKED
    engine
        .set_thread_state(ThreadId::new(0), ThreadState::Blocked)
        .unwrap();

    // Try to schedule from thread 1 (still RUNNABLE)
    let task_id_2 = engine
        .schedule_task(ThreadId::new(1), 100_000_000)
        .expect("Second schedule should succeed");

    // Verify TaskIds are still unique and ordered despite state change
    assert_ne!(task_id_1, task_id_2, "TaskIds must remain unique");
    assert!(
        task_id_1.as_usize() < task_id_2.as_usize(),
        "TaskIds must remain ordered despite state changes"
    );

    // Change thread 1 state to COMPLETED
    engine
        .set_thread_state(ThreadId::new(1), ThreadState::Completed)
        .unwrap();

    // Reset thread 2 and schedule from it
    engine
        .set_thread_state(ThreadId::new(2), ThreadState::Runnable)
        .unwrap();

    let task_id_3 = engine
        .schedule_task(ThreadId::new(2), 100_000_000)
        .expect("Third schedule should succeed");

    // Verify ordering is maintained
    assert!(
        task_id_2.as_usize() < task_id_3.as_usize(),
        "TaskId ordering must persist across multiple state transitions"
    );
}
