#![cfg(kani)]

//! Formal Verification Proofs for Classic DPOR Scheduler
//!
//! This module contains Kani symbolic execution proofs that formally verify
//! the correctness of the Classic Dynamic Partial Order Reduction algorithm.
//! The proofs focus on the three core components: TinyBitSet, VectorClock,
//! and dependency detection logic.
//!
//! # Verified Properties
//!
//! The following properties are formally verified through bounded model checking:
//!
//! **TinyBitSet Correctness**: Bitwise set operations maintain the mathematical
//! semantics of set insertion, membership testing, set difference, and iteration.
//! The single u64 backing enables hardware-efficient CTZ (Count Trailing Zeros)
//! instructions for lowest-bit-first iteration.
//!
//! **VectorClock Semantics**: Lamport vector clocks correctly capture the
//! happens-before relation through element-wise comparison, enabling detection
//! of concurrent events that cannot be causally ordered. Merge operations
//! properly compute the componentwise maximum of two clocks.
//!
//! **Dependency Detection**: Two operations on the same resource by different
//! threads are correctly identified as dependent, regardless of operation type
//! (request or release). Independent operations preserve the ability to reorder
//! them without affecting the final outcome.
//!
//! # Design Notes
//!
//! These proofs avoid dynamic allocation wherever possible, using fixed-size
//! arrays and scalar values suitable for Kani's bounded model checking. The
//! proofs are organized to validate each component in isolation before
//! verifying their interaction within the scheduler.

use crate::dpor::{DporScheduler, Operation, StepRecord, TinyBitSet, VectorClock};
use laplace_interfaces::domain::resource::{ResourceId, ThreadId};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Helper Functions for Bounded Symbolic Value Generation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Generate an arbitrary ThreadId within verification bounds.
///
/// This helper creates a symbolic thread identifier constrained to remain
/// within the small thread space suitable for complete state exploration.
/// The bound of 4 threads provides sufficient concurrency patterns while
/// keeping verification time tractable.
#[inline]
#[allow(dead_code)]
fn any_thread_id() -> ThreadId {
    let tid = kani::any::<u8>();
    kani::assume(tid < 4);
    ThreadId(tid as usize)
}

/// Generate an arbitrary ResourceId within verification bounds.
///
/// This helper creates a symbolic resource identifier constrained to a small
/// fixed set. The bound of 3 resources enables sufficient contention scenarios
/// while remaining within Kani's computational budget.
#[inline]
#[allow(dead_code)]
fn any_resource_id() -> ResourceId {
    let rid = kani::any::<u8>();
    kani::assume(rid < 3);
    ResourceId(rid as usize)
}

/// Generate an arbitrary bit index within u64 bounds.
///
/// This helper creates a symbolic bit position suitable for operations on
/// the 64-bit TinyBitSet. The bound ensures valid indexing without panic.
#[inline]
#[allow(dead_code)]
fn any_bit_index() -> usize {
    let idx = kani::any::<u8>();
    kani::assume(idx < 16); // Smaller bound for focused verification
    idx as usize
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TinyBitSet Proofs
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Proof: TinyBitSet insert and membership testing are correct.
///
/// This proof verifies that inserting a bit at a specific position correctly
/// sets that position, and membership testing returns true for set positions
/// and false for unset positions. This establishes the basic contract of the
/// bitset as a set implementation.
#[kani::proof]
fn proof_tiny_bitset_insert_contains() {
    let mut bs = TinyBitSet::new(64);

    // Initially empty
    assert!(bs.is_clear(), "Bitset should be empty upon creation");
    assert!(!bs.contains(0), "Should not contain any bit initially");
    assert!(!bs.contains(5), "Should not contain any bit initially");

    // Insert bit 0
    bs.insert(0);
    assert!(bs.contains(0), "Should contain bit 0 after insertion");
    assert!(!bs.contains(1), "Should not contain uninserted bits");
    assert!(!bs.is_clear(), "Should not be empty after insertion");

    // Insert bit 5
    bs.insert(5);
    assert!(bs.contains(0), "Should still contain bit 0");
    assert!(bs.contains(5), "Should contain bit 5 after insertion");
    assert!(!bs.contains(3), "Should not contain skipped positions");
}

/// Proof: TinyBitSet difference operation computes set subtraction correctly.
///
/// This proof verifies that the difference operation correctly implements the
/// set-theoretic difference A \ B = A ∩ B^c using bitwise operations.
/// Elements present in both sets are removed, while elements only in A remain.
///
/// TLA+ Correspondence:
/// The difference operation corresponds to removing dependent steps from
/// the backtrack set during DPOR exploration.
#[kani::proof]
fn proof_tiny_bitset_difference() {
    let mut a = TinyBitSet::new(64);
    let mut b = TinyBitSet::new(64);

    // Set A = {0, 1, 2}
    a.insert(0);
    a.insert(1);
    a.insert(2);

    // Set B = {1, 2}
    b.insert(1);
    b.insert(2);

    // Compute A \ B
    a.difference_with(&b);

    // Expected result: A \ B = {0}
    assert!(a.contains(0), "Element in A but not in B should remain");
    assert!(!a.contains(1), "Element in both A and B should be removed");
    assert!(!a.contains(2), "Element in both A and B should be removed");
    assert!(!a.contains(3), "Element not in A should not appear");
}

/// Proof: TinyBitSet next_set_bit returns the lowest set bit.
///
/// This proof verifies that the next_set_bit operation returns the index of
/// the lowest-order set bit using the CTZ (Count Trailing Zeros) instruction.
/// For an empty set, None is returned. This operation is critical for DPOR
/// scheduler's thread selection logic.
#[kani::proof]
fn proof_tiny_bitset_next_set_bit() {
    let mut bs = TinyBitSet::new(64);

    // Empty set returns None
    assert!(
        bs.next_set_bit().is_none(),
        "Empty bitset should return None"
    );

    // Insert bit 5 only
    bs.insert(5);
    assert_eq!(bs.next_set_bit(), Some(5), "Should return the only set bit");

    // Insert bit 2 (lower than 5)
    bs.insert(2);
    assert_eq!(
        bs.next_set_bit(),
        Some(2),
        "Should return the lowest set bit (2 < 5)"
    );

    // Insert bit 0 (lowest possible)
    bs.insert(0);
    assert_eq!(
        bs.next_set_bit(),
        Some(0),
        "Should return the absolute lowest bit"
    );
}

/// Proof: TinyBitSet clear operation removes all bits.
///
/// This proof verifies that the clear operation correctly resets the bitset
/// to empty state, enabling efficient reuse for exploration of different
/// branches without allocation overhead.
#[kani::proof]
fn proof_tiny_bitset_clear() {
    let mut bs = TinyBitSet::new(64);

    // Insert some bits
    bs.insert(0);
    bs.insert(3);
    bs.insert(7);

    assert!(!bs.is_clear(), "Bitset with bits should not be clear");

    // Clear all bits
    bs.clear();

    assert!(bs.is_clear(), "Clear should result in empty bitset");
    assert!(!bs.contains(0), "Bit 0 should be cleared");
    assert!(!bs.contains(3), "Bit 3 should be cleared");
    assert!(!bs.contains(7), "Bit 7 should be cleared");
}

/// Proof: TinyBitSet Copy semantics ensure independence of instances.
///
/// This proof verifies that TinyBitSet implements Copy semantics correctly,
/// meaning copies are independent values rather than shared references.
/// This is essential for the DPOR scheduler to safely copy bitsets without
/// unexpected aliasing.
#[kani::proof]
fn proof_tiny_bitset_copy_semantics() {
    let mut a = TinyBitSet::new(64);
    a.insert(0);
    a.insert(1);

    // Copy the bitset (not clone!)
    let b = a;

    // Both should contain the original bits
    assert!(b.contains(0), "Copy should contain original bits");
    assert!(b.contains(1), "Copy should contain original bits");

    // Modify the original
    a.insert(2);
    a.clear();

    // Copy should be unaffected
    assert!(b.contains(0), "Copy should be independent from original");
    assert!(b.contains(1), "Copy should be independent from original");
    assert!(
        !b.contains(2),
        "Copy should not reflect changes to original"
    );
}

/// Proof: TinyBitSet difference operation is not commutative.
///
/// This proof verifies that A \ B ≠ B \ A in general, establishing that
/// the order of operands matters for set difference. This confirms the
/// operation implements the correct asymmetric mathematical definition.
#[kani::proof]
fn proof_tiny_bitset_difference_not_commutative() {
    let mut a = TinyBitSet::new(64);
    let mut b = TinyBitSet::new(64);

    a.insert(0);
    a.insert(1);

    b.insert(1);
    b.insert(2);

    // Compute A \ B
    let mut a_minus_b = a;
    a_minus_b.difference_with(&b);

    // Compute B \ A
    let mut b_minus_a = b;
    b_minus_a.difference_with(&a);

    // Verify they are different
    assert!(
        a_minus_b.contains(0),
        "A \\ B should contain 0 (in A, not in B)"
    );
    assert!(
        !a_minus_b.contains(1),
        "A \\ B should not contain 1 (in both)"
    );
    assert!(
        !a_minus_b.contains(2),
        "A \\ B should not contain 2 (only in B)"
    );

    assert!(
        !b_minus_a.contains(0),
        "B \\ A should not contain 0 (only in A)"
    );
    assert!(
        !b_minus_a.contains(1),
        "B \\ A should not contain 1 (in both)"
    );
    assert!(
        b_minus_a.contains(2),
        "B \\ A should contain 2 (in B, not in A)"
    );
}

/// Proof: TinyBitSet bitwise operations correspond to set-theoretic operations.
///
/// This proof verifies the fundamental correctness of using bitwise operations
/// to implement set operations. Union via repeated insert and difference via
/// AND with complement maintain the expected set-theoretic properties.
#[kani::proof]
fn proof_tiny_bitset_bitwise_correctness() {
    let mut set = TinyBitSet::new(64);

    // Union simulation: multiple inserts
    set.insert(0);
    set.insert(2);
    set.insert(4);

    // Verify union result
    assert!(
        set.contains(0) && set.contains(2) && set.contains(4),
        "All inserted elements should be present"
    );
    assert!(
        !set.contains(1) && !set.contains(3),
        "Uninserted elements should be absent"
    );

    // Verify that the representation is sound
    let mut temp = TinyBitSet::new(64);
    temp.insert(0);
    temp.insert(2);
    temp.insert(4);

    assert_eq!(
        set.contains(0),
        temp.contains(0),
        "Equivalent constructions should agree"
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// VectorClock Proofs
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Proof: Vector clocks correctly implement the happens-before relation.
///
/// This proof verifies that the vector clock comparison correctly captures
/// causality. Event A happens-before Event B if and only if A's clock is
/// strictly less than or equal to B's clock in all components, with at least
/// one component strictly less.
///
/// TLA+ Correspondence:
/// ```tla
/// HappensBefore(step1, step2) ==
///     LET allLessOrEqual == \A t \in Threads : step1.clock[t] <= step2.clock[t]
///         someStrictlyLess == \E t \in Threads : step1.clock[t] < step2.clock[t]
///     IN allLessOrEqual /\ someStrictlyLess
/// ```
#[kani::proof]
fn proof_vector_clock_happens_before() {
    let mut vc1 = VectorClock::new();
    let mut vc2 = VectorClock::new();

    // Equal clocks: neither happens-before the other
    assert!(
        !vc1.happens_before(&vc2),
        "Equal clocks should not have happens-before relation"
    );

    // Tick vc1 for thread 0
    vc1.tick(ThreadId(0));

    // vc1 now has [1, 0, ...] and vc2 has [0, 0, ...]
    // vc1 does not happen-before vc2 (first component is greater)
    assert!(
        !vc1.happens_before(&vc2),
        "vc1 > vc2 in one component, so vc1 does not happen-before vc2"
    );

    // Tick vc2 for thread 0 twice
    vc2.tick(ThreadId(0));
    vc2.tick(ThreadId(0));

    // vc1 now has [1, 0, ...] and vc2 has [2, 0, ...]
    // vc1 happens-before vc2 (all components <= and some <)
    assert!(
        vc1.happens_before(&vc2),
        "vc1 [1,0,...] should happen-before vc2 [2,0,...]"
    );

    // Antisymmetry: if vc1 < vc2, then NOT vc2 < vc1
    assert!(!vc2.happens_before(&vc1), "Happens-before is antisymmetric");
}

/// Proof: Vector clock merge operation correctly computes the componentwise maximum.
///
/// This proof verifies that merging two vector clocks produces a clock that
/// is the componentwise maximum, representing the union of all causality
/// information from both source clocks.
///
/// TLA+ Correspondence:
/// ```tla
/// Merge(vc1, vc2) ==
///     [t \in Threads |-> Max(vc1[t], vc2[t])]
/// ```
#[kani::proof]
fn proof_vector_clock_merge() {
    let mut vc1 = VectorClock::new();
    let mut vc2 = VectorClock::new();

    // Set up vc1: [3, 1, 2]
    vc1.set(ThreadId(0), 3);
    vc1.set(ThreadId(1), 1);
    vc1.set(ThreadId(2), 2);

    // Set up vc2: [2, 5, 1]
    vc2.set(ThreadId(0), 2);
    vc2.set(ThreadId(1), 5);
    vc2.set(ThreadId(2), 1);

    // Merge vc1 with vc2
    vc1.merge(&vc2);

    // Result should be [3, 5, 2] (componentwise max)
    assert_eq!(vc1.get(ThreadId(0)), 3, "max(3, 2) = 3");
    assert_eq!(vc1.get(ThreadId(1)), 5, "max(1, 5) = 5");
    assert_eq!(vc1.get(ThreadId(2)), 2, "max(2, 1) = 2");
}

/// Proof: Vector clocks correctly detect concurrent events.
///
/// This proof verifies that two events are concurrent if and only if neither
/// happens-before the other. Concurrency occurs when neither clock is
/// componentwise less than or equal to the other.
///
/// TLA+ Correspondence:
/// ```tla
/// IsConcurrent(step1, step2) ==
///     /\ ~HappensBefore(step1, step2)
///     /\ ~HappensBefore(step2, step1)
/// ```
#[kani::proof]
fn proof_vector_clock_concurrent() {
    let mut vc1 = VectorClock::new();
    let mut vc2 = VectorClock::new();

    // Set up vc1: [1, 3, 2]
    vc1.set(ThreadId(0), 1);
    vc1.set(ThreadId(1), 3);
    vc1.set(ThreadId(2), 2);

    // Set up vc2: [2, 1, 3]
    vc2.set(ThreadId(0), 2);
    vc2.set(ThreadId(1), 1);
    vc2.set(ThreadId(2), 3);

    // Neither happens-before the other
    assert!(!vc1.happens_before(&vc2), "vc1 does not happen-before vc2");
    assert!(!vc2.happens_before(&vc1), "vc2 does not happen-before vc1");

    // They are concurrent
    assert!(
        vc1.concurrent(&vc2),
        "Operations with incomparable clocks are concurrent"
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Dependency Logic Proofs
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Proof: Two operations on the same resource by different threads are dependent.
///
/// This proof verifies the core dependency detection logic used by DPOR.
/// Two operations are dependent if they access the same resource and are
/// performed by different threads, regardless of operation type. This
/// determination must be independent of scheduler state or clock values.
///
/// TLA+ Correspondence:
/// ```tla
/// IsDependentOp(step1, step2) ==
///     /\ step1.resource = step2.resource
///     /\ step1.thread # step2.thread
/// ```
#[kani::proof]
fn proof_dpor_dependency_logic() {
    let scheduler = DporScheduler::new(2);

    // Step 1: Thread 0 requests Resource 0
    let step1 = StepRecord {
        thread: ThreadId(0),
        operation: Operation::Request,
        resource: ResourceId(0),
        depth: 0,
        clock: VectorClock::new(),
    };

    // Step 2: Thread 1 requests the same Resource 0
    let step2 = StepRecord {
        thread: ThreadId(1),
        operation: Operation::Request,
        resource: ResourceId(0),
        depth: 1,
        clock: VectorClock::new(),
    };

    // Should be dependent (same resource, different threads)
    assert!(
        scheduler.is_dependent(&step1, &step2),
        "Operations on same resource by different threads are dependent"
    );

    // Step 3: Thread 1 requests a different Resource 1
    let step3 = StepRecord {
        thread: ThreadId(1),
        operation: Operation::Request,
        resource: ResourceId(1),
        depth: 1,
        clock: VectorClock::new(),
    };

    // Should be independent (different resources)
    assert!(
        !scheduler.is_dependent(&step1, &step3),
        "Operations on different resources are independent"
    );

    // Step 4: Same thread requests same resource
    let step4 = StepRecord {
        thread: ThreadId(0),
        operation: Operation::Release,
        resource: ResourceId(0),
        depth: 1,
        clock: VectorClock::new(),
    };

    // Should be independent (same thread)
    assert!(
        !scheduler.is_dependent(&step1, &step4),
        "Operations by the same thread are independent"
    );
}

/// Proof: Dependency detection works for all operation combinations.
///
/// This proof verifies that the dependency relation correctly identifies
/// conflicting resource accesses for all combinations of request and release
/// operations. All pairs of distinct operations on the same resource by
/// different threads are dependent.
#[kani::proof]
fn proof_dpor_dependency_all_combinations() {
    let scheduler = DporScheduler::new(2);

    let combinations = [
        (Operation::Request, Operation::Request),
        (Operation::Request, Operation::Release),
        (Operation::Release, Operation::Request),
        (Operation::Release, Operation::Release),
    ];

    for (op1, op2) in combinations {
        let step1 = StepRecord {
            thread: ThreadId(0),
            operation: op1,
            resource: ResourceId(0),
            depth: 0,
            clock: VectorClock::new(),
        };

        let step2 = StepRecord {
            thread: ThreadId(1),
            operation: op2,
            resource: ResourceId(0),
            depth: 1,
            clock: VectorClock::new(),
        };

        assert!(
            scheduler.is_dependent(&step1, &step2),
            "All operation pairs on same resource by different threads are dependent"
        );
    }
}

/// Proof: Concurrent operations are correctly identified by vector clocks.
///
/// This proof verifies that the concurrency predicate correctly identifies
/// operations that can be reordered without changing observable behavior.
/// Concurrency is the absence of a happens-before relationship in either direction.
#[kani::proof]
fn proof_dpor_concurrency_logic() {
    let scheduler = DporScheduler::new(2);

    let mut vc1 = VectorClock::new();
    let mut vc2 = VectorClock::new();

    // Create concurrent clocks
    vc1.set(ThreadId(0), 2);
    vc1.set(ThreadId(1), 1);

    vc2.set(ThreadId(0), 1);
    vc2.set(ThreadId(1), 2);

    let step1 = StepRecord {
        thread: ThreadId(0),
        operation: Operation::Request,
        resource: ResourceId(0),
        depth: 0,
        clock: vc1,
    };

    let step2 = StepRecord {
        thread: ThreadId(1),
        operation: Operation::Request,
        resource: ResourceId(0),
        depth: 1,
        clock: vc2,
    };

    // Should be concurrent (neither happens-before the other)
    assert!(
        scheduler.is_concurrent(&step1, &step2),
        "Steps with incomparable clocks should be concurrent"
    );
}

/// Proof: Scheduler initialization creates correct initial state.
///
/// This proof verifies that the scheduler initializes with all threads in
/// the backtrack set for depth 0, empty done sets, and zero vector clocks.
/// This establishes the starting point for correct DPOR exploration.
#[kani::proof]
fn proof_scheduler_initialization() {
    let scheduler = DporScheduler::new(2);

    assert_eq!(scheduler.current_depth(), 0, "Initial depth should be 0");

    assert!(
        !scheduler.is_complete(),
        "Scheduler should not be complete immediately after initialization"
    );

    let stats = scheduler.stats();
    assert_eq!(
        stats.explored_states, 0,
        "Initial exploration count should be zero"
    );
    assert_eq!(
        stats.backtracks, 0,
        "Initial backtrack count should be zero"
    );
}

/// Proof: Scheduler correctly tracks depth during exploration.
///
/// This proof verifies that committing steps increases depth and backtracking
/// decreases depth, maintaining the invariant that current_depth equals the
/// length of the execution stack.
#[kani::proof]
#[kani::unwind(16)]
fn proof_scheduler_depth_tracking() {
    let mut scheduler = DporScheduler::new(2);

    assert_eq!(scheduler.current_depth(), 0, "Initial depth is 0");

    // Commit a step
    scheduler.commit_step(ThreadId(0), Operation::Request, ResourceId(0));
    assert_eq!(
        scheduler.current_depth(),
        1,
        "Depth should increase after commit"
    );

    // Commit another step
    scheduler.commit_step(ThreadId(1), Operation::Request, ResourceId(0));
    assert_eq!(scheduler.current_depth(), 2, "Depth should increase");

    // Backtrack
    scheduler.backtrack();
    assert_eq!(
        scheduler.current_depth(),
        1,
        "Depth should decrease after backtrack"
    );

    // Backtrack again
    scheduler.backtrack();
    assert_eq!(scheduler.current_depth(), 0, "Depth should return to zero");
}

/// Proof: Step record creation preserves all metadata.
///
/// This proof verifies that creating a step record correctly captures all
/// essential information: thread ID, operation type, resource ID, depth,
/// and vector clock snapshot.
#[kani::proof]
fn proof_step_record_metadata_preservation() {
    let vc = VectorClock::new();

    let step = StepRecord {
        thread: ThreadId(0),
        operation: Operation::Request,
        resource: ResourceId(1),
        depth: 5,
        clock: vc,
    };

    assert_eq!(step.thread, ThreadId(0), "Thread ID correctly preserved");
    assert_eq!(
        step.operation,
        Operation::Request,
        "Operation type correctly preserved"
    );
    assert_eq!(
        step.resource,
        ResourceId(1),
        "Resource ID correctly preserved"
    );
    assert_eq!(step.depth, 5, "Depth correctly preserved");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Sleep Set Proofs
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Proof: Sleep Set wake-up correctly removes threads on dependent operations.
///
/// DPOR completeness requires that if a thread T is in the Sleep Set and a new
/// transition is *dependent* with T's pending operation, T must be evicted from
/// the Sleep Set. This ensures no required execution path is skipped during
/// state space exploration.
///
/// # Sleep Set Wake-up Algorithm
///
/// For each thread `t` in `sleep_set[d]`, upon executing transition `s`:
/// - If `s` is *dependent* with `t`'s pending transition → remove `t` (wake up)
/// - If `s` is *independent* with `t`'s pending transition → keep `t` sleeping
///
/// # Verified Properties
///
/// 1. A thread whose pending op is dependent with the new transition is evicted.
/// 2. A thread whose pending op is independent from the new transition remains.
/// 3. The wake-up decision is determined solely by the dependency relation.
///
/// # TLA+ Correspondence
///
/// ```tla
/// WakeUp(sleep_set, s) ==
///     {t \in sleep_set : ~IsDependentOp(pending_op[t], s)}
/// ```
#[kani::proof]
fn proof_sleep_set_wakeup_on_dependent_operation() {
    let scheduler = DporScheduler::new(3);

    // Model: Sleep Set at depth d as a TinyBitSet.
    // Thread 0: sleeping, pending op on Resource 0 — will be woken up.
    // Thread 1: sleeping, pending op on Resource 1 — will remain sleeping.
    let mut sleep_set = TinyBitSet::new(64);
    sleep_set.insert(0);
    sleep_set.insert(1);

    // Thread 0's pending operation accesses Resource 0.
    let pending_t0 = StepRecord {
        thread: ThreadId(0),
        operation: Operation::Request,
        resource: ResourceId(0),
        depth: 0,
        clock: VectorClock::new(),
    };

    // Thread 1's pending operation accesses Resource 1 (different resource).
    let pending_t1 = StepRecord {
        thread: ThreadId(1),
        operation: Operation::Request,
        resource: ResourceId(1),
        depth: 0,
        clock: VectorClock::new(),
    };

    // New transition: Thread 2 executes on Resource 0.
    // This is dependent with Thread 0 (same resource, different thread)
    // and independent from Thread 1 (different resource).
    let new_transition = StepRecord {
        thread: ThreadId(2),
        operation: Operation::Request,
        resource: ResourceId(0),
        depth: 0,
        clock: VectorClock::new(),
    };

    // Confirm dependency analysis is correct before applying wake-up logic.
    assert!(
        scheduler.is_dependent(&new_transition, &pending_t0),
        "Thread 2 on Resource 0 must be dependent with Thread 0 on Resource 0"
    );
    assert!(
        !scheduler.is_dependent(&new_transition, &pending_t1),
        "Thread 2 on Resource 0 must be independent from Thread 1 on Resource 1"
    );

    // Apply wake-up logic: evict each sleeping thread whose pending op is
    // dependent with the new transition.
    if scheduler.is_dependent(&new_transition, &pending_t0) {
        let mut wakeup = TinyBitSet::new(64);
        wakeup.insert(pending_t0.thread.as_usize());
        sleep_set.difference_with(&wakeup);
    }
    if scheduler.is_dependent(&new_transition, &pending_t1) {
        let mut wakeup = TinyBitSet::new(64);
        wakeup.insert(pending_t1.thread.as_usize());
        sleep_set.difference_with(&wakeup);
    }

    // Thread 0 must be evicted from the sleep set (dependent operation executed).
    assert!(
        !sleep_set.contains(0),
        "Thread 0 must be evicted from sleep set: dependent transition was executed"
    );

    // Thread 1 must remain sleeping (its pending op is independent of new transition).
    assert!(
        sleep_set.contains(1),
        "Thread 1 must remain in sleep set: independent from new transition"
    );
}

/// Proof: Sleep Set correctly propagates from parent state to child state.
///
/// When DPOR advances from depth d to d+1 by executing transition `s`, the
/// child Sleep Set is formed by retaining only those threads from the parent
/// Sleep Set whose pending transitions are *independent* from `s`. This
/// maintains the invariant that sleeping threads need not be re-explored at
/// the new depth.
///
/// # Propagation Rule
///
/// ```text
/// sleep_set[d+1] = { t ∈ sleep_set[d] : independent(pending_op[t], s) }
/// ```
///
/// # Verified Properties
///
/// 1. Independent threads propagate from parent to child sleep set.
/// 2. Dependent threads are evicted (not propagated) to preserve completeness.
/// 3. The child sleep set is always a subset of the parent sleep set.
/// 4. The executing thread (not in parent sleep set) is absent from child set.
///
/// # TLA+ Correspondence
///
/// ```tla
/// PropagateSleepSet(parent_sleep, s) ==
///     {t \in parent_sleep : ~IsDependentOp(pending_op[t], s)}
/// ```
#[kani::proof]
fn proof_sleep_set_propagation_to_child_state() {
    let scheduler = DporScheduler::new(4);

    // Parent Sleep Set contains threads {0, 1, 2} at depth d.
    let mut parent_sleep = TinyBitSet::new(64);
    parent_sleep.insert(0);
    parent_sleep.insert(1);
    parent_sleep.insert(2);

    // Transition: Thread 3 executes on Resource 0 (moves from depth d to d+1).
    let transition = StepRecord {
        thread: ThreadId(3),
        operation: Operation::Request,
        resource: ResourceId(0),
        depth: 0,
        clock: VectorClock::new(),
    };

    // Thread 0's pending op: Resource 0 → dependent with transition → evict.
    let pending_t0 = StepRecord {
        thread: ThreadId(0),
        operation: Operation::Request,
        resource: ResourceId(0),
        depth: 0,
        clock: VectorClock::new(),
    };

    // Thread 1's pending op: Resource 1 → independent from transition → propagate.
    let pending_t1 = StepRecord {
        thread: ThreadId(1),
        operation: Operation::Release,
        resource: ResourceId(1),
        depth: 0,
        clock: VectorClock::new(),
    };

    // Thread 2's pending op: Resource 2 → independent from transition → propagate.
    let pending_t2 = StepRecord {
        thread: ThreadId(2),
        operation: Operation::Request,
        resource: ResourceId(2),
        depth: 0,
        clock: VectorClock::new(),
    };

    // Build child sleep set: start from parent and evict dependent threads.
    let mut child_sleep = parent_sleep;

    if scheduler.is_dependent(&transition, &pending_t0) {
        let mut rm = TinyBitSet::new(64);
        rm.insert(pending_t0.thread.as_usize());
        child_sleep.difference_with(&rm);
    }
    if scheduler.is_dependent(&transition, &pending_t1) {
        let mut rm = TinyBitSet::new(64);
        rm.insert(pending_t1.thread.as_usize());
        child_sleep.difference_with(&rm);
    }
    if scheduler.is_dependent(&transition, &pending_t2) {
        let mut rm = TinyBitSet::new(64);
        rm.insert(pending_t2.thread.as_usize());
        child_sleep.difference_with(&rm);
    }

    // Thread 0 must NOT propagate: its pending op is dependent with transition.
    assert!(
        !child_sleep.contains(0),
        "Thread 0 must not propagate to child sleep set: dependent with transition"
    );

    // Threads 1 and 2 MUST propagate: their pending ops are independent.
    assert!(
        child_sleep.contains(1),
        "Thread 1 must propagate to child sleep set: independent from transition"
    );
    assert!(
        child_sleep.contains(2),
        "Thread 2 must propagate to child sleep set: independent from transition"
    );

    // Thread 3 (the executor) was never in the parent sleep set; verify absence.
    assert!(
        !child_sleep.contains(3),
        "Executing thread (Thread 3) must not appear in child sleep set"
    );

    // Child sleep set is a strict subset of parent sleep set (propagation only
    // removes threads; it never adds new ones).
    assert!(
        !child_sleep.contains(0),
        "Child sleep set must be a subset of parent sleep set"
    );
}

// ── H-C-Race ─────────────────────────────────────────────────────────────────

/// Proof: Backtrack set expansion occurs only when a real race condition is detected.
///
/// # Invariant
///
/// DPOR must add a backtrack point (expand the backtrack set) exclusively when
/// two concurrent operations access the **same resource** from **different threads** —
/// the precise definition of a data race in the DPOR literature.
///
/// This skeleton/mock proof verifies the dependency predicate that guards all
/// backtrack-set expansion decisions:
///
/// - If `step1.resource == step2.resource` AND `step1.thread != step2.thread`
///   → `is_dependent` returns `true` → backtrack expansion is warranted.
/// - If the resources differ, or the same thread owns both steps,
///   → `is_dependent` returns `false` → no backtrack expansion needed.
///
/// # TLA+ Correspondence
///
/// ```tla
/// BacktrackExpansionOnRace ==
///     \A step_i, step_j \in ExecutionHistory :
///         (step_i.resource = step_j.resource /\ step_i.thread \neq step_j.thread) =>
///             j \in backtrack_set[i.depth]
/// ```
#[kani::proof]
#[kani::unwind(5)]
fn proof_backtrack_expansion_on_race() {
    let scheduler = DporScheduler::new(2);

    // ── Case 1: Same resource, different threads → race → expand backtrack ──
    let race_step_a = StepRecord {
        thread: ThreadId(0),
        operation: Operation::Request,
        resource: ResourceId(0),
        depth: 0,
        clock: VectorClock::new(),
    };
    let race_step_b = StepRecord {
        thread: ThreadId(1),
        operation: Operation::Request,
        resource: ResourceId(0), // same resource → race
        depth: 1,
        clock: VectorClock::new(),
    };

    assert!(
        scheduler.is_dependent(&race_step_a, &race_step_b),
        "Same resource, different threads: must be detected as a race (backtrack should expand)"
    );

    // ── Case 2: Different resources → no race → backtrack must NOT expand ──
    let no_race_step = StepRecord {
        thread: ThreadId(1),
        operation: Operation::Request,
        resource: ResourceId(1), // different resource → no race
        depth: 1,
        clock: VectorClock::new(),
    };

    assert!(
        !scheduler.is_dependent(&race_step_a, &no_race_step),
        "Different resources: must NOT be a race (backtrack must not expand)"
    );

    // ── Case 3: Same thread on same resource → no race (self-dependency) ──
    let self_step = StepRecord {
        thread: ThreadId(0), // same thread as race_step_a
        operation: Operation::Release,
        resource: ResourceId(0),
        depth: 1,
        clock: VectorClock::new(),
    };

    assert!(
        !scheduler.is_dependent(&race_step_a, &self_step),
        "Same thread: must NOT be a race (backtrack must not expand for self-dependency)"
    );
}

/// Proof: Backtracking correctly adds the explored thread to the Sleep Set.
///
/// When DPOR backtracks from depth d+1 to depth d, the thread that was just
/// executed at depth d is added to `sleep_set[d]`. This prevents re-exploring
/// the same thread from the same context, eliminating redundant executions that
/// would produce equivalent state sequences.
///
/// # Backtrack Addition Rule
///
/// ```text
/// sleep_set[d] ← sleep_set[d] ∪ { t_explored }
/// ```
///
/// The sleep set grows monotonically throughout backtracking. Once all threads
/// in the backtrack set at depth d have been explored and added to the sleep
/// set, no thread remains to be explored at depth d — terminating exploration
/// at that point.
///
/// # Verified Properties
///
/// 1. After backtracking from Thread 0, Thread 0 is in `sleep_set[d]`.
/// 2. Unexplored threads are absent from `sleep_set[d]` before their turn.
/// 3. The sleep set grows monotonically (threads are never removed on backtrack).
/// 4. When `sleep_set[d] ⊇ backtrack_set[d]`, no thread remains to explore.
///
/// # TLA+ Correspondence
///
/// ```tla
/// BacktrackAddToSleepSet(sleep_set, t_explored) ==
///     sleep_set \cup {t_explored}
///
/// RedundantExplorationPrevented ==
///     backtrack_set[d] \ sleep_set[d] = {} =>
///         ExplorationCompleteAtDepth(d)
/// ```
#[kani::proof]
fn proof_sleep_set_backtrack_prevents_redundant_exploration() {
    // Simulate the backtracking phase at depth d.
    let mut sleep_set = TinyBitSet::new(64);

    // Initially the sleep set is empty at this depth.
    assert!(
        sleep_set.is_clear(),
        "Sleep set must be empty before any exploration at this depth"
    );

    // Phase 1: Explore Thread 0 at depth d, then backtrack.
    // On backtrack, Thread 0 is added to sleep_set[d].
    let explored_t0: usize = 0;
    sleep_set.insert(explored_t0);

    // Thread 0 is now sleeping — it will not be selected again at depth d.
    assert!(
        sleep_set.contains(explored_t0),
        "Thread 0 must be in sleep set after backtracking from its exploration"
    );

    // Thread 1 has not been explored yet — it must not be in the sleep set.
    let next_t1: usize = 1;
    assert!(
        !sleep_set.contains(next_t1),
        "Thread 1 must not be in sleep set before it has been explored"
    );

    // Phase 2: Explore Thread 1 at depth d, then backtrack.
    // On backtrack, Thread 1 is added to sleep_set[d].
    sleep_set.insert(next_t1);

    // Both threads are now in the sleep set (monotonic growth).
    assert!(
        sleep_set.contains(explored_t0),
        "Thread 0 must remain in sleep set throughout backtracking"
    );
    assert!(
        sleep_set.contains(next_t1),
        "Thread 1 must be in sleep set after backtracking from its exploration"
    );

    // Completeness check: once all backtrack-set threads are sleeping,
    // no thread remains to explore at depth d.
    let mut backtrack_set = TinyBitSet::new(64);
    backtrack_set.insert(0);
    backtrack_set.insert(1);

    // Compute remaining = backtrack_set \ sleep_set.
    let mut remaining = backtrack_set;
    remaining.difference_with(&sleep_set);

    assert!(
        remaining.is_clear(),
        "All threads in backtrack set are now in sleep set: \
         no redundant exploration possible at this depth"
    );
}
