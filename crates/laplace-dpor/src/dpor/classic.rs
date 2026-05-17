//! DPOR Scheduler Implementation (Classic Algorithm)
//!
//! This module implements the core Classic DPOR algorithm for state space
//! exploration with a custom zero-allocation bitset.

use super::vector_clock::VectorClock;
use super::{MAX_DEPTH, MAX_THREADS};
use laplace_interfaces::domain::resource::{ResourceId, ThreadId};
use std::fmt;

/// Tiny bitset backed by a single u64
///
/// # Design
///
/// - **Capacity:** 64 bits (perfect for MAX_THREADS = 8)
/// - **Storage:** Single u64 on stack/register
/// - **Cost:** Zero heap allocation
/// - **Kani:** Simple bitwise ops (no SIMD loops)
///
/// # Operations
///
/// All operations are O(1) using hardware bitwise instructions.
///
/// ```text
/// insert(3):     bits |= (1 << 3)
/// contains(3):   bits & (1 << 3)
/// difference:    A & !B
/// next_set:      trailing_zeros() (CTZ instruction)
/// ```
#[derive(Copy, Clone, PartialEq, Eq, Default)]
pub struct TinyBitSet {
    bits: u64,
}

impl TinyBitSet {
    /// Create a new empty bitset
    ///
    /// # Arguments
    ///
    /// * `capacity` - Maximum number of bits (ignored, always 64)
    #[inline(always)]
    pub const fn new(_capacity: usize) -> Self {
        Self { bits: 0 }
    }

    /// Insert a bit at the given index
    ///
    /// # Panics
    ///
    /// Panics if `idx >= 64`
    #[inline(always)]
    pub fn insert(&mut self, idx: usize) {
        assert!(idx < 64, "Index out of bounds: {}", idx);
        self.bits |= 1u64 << idx;
    }

    /// Check if a bit is set
    ///
    /// # Panics
    ///
    /// Panics if `idx >= 64`
    #[inline(always)]
    pub fn contains(&self, idx: usize) -> bool {
        assert!(idx < 64, "Index out of bounds: {}", idx);
        (self.bits & (1u64 << idx)) != 0
    }

    /// Compute difference with another bitset (A \ B = A & !B)
    ///
    /// This modifies `self` to be the set difference.
    ///
    /// # Example
    ///
    /// ```text
    /// A = {0, 1, 2}  -> 0b111
    /// B = {1, 2}     -> 0b110
    /// A \ B = {0}    -> 0b001
    /// ```
    #[inline(always)]
    pub fn difference_with(&mut self, other: &Self) {
        self.bits &= !other.bits;
    }

    /// Find the next set bit (returns the index)
    ///
    /// Returns `None` if no bits are set.
    ///
    /// # Implementation
    ///
    /// Uses the `trailing_zeros()` intrinsic which compiles to a single
    /// CTZ (Count Trailing Zeros) instruction on x86/ARM.
    #[inline(always)]
    pub fn next_set_bit(&self) -> Option<usize> {
        if self.bits == 0 {
            None
        } else {
            Some(self.bits.trailing_zeros() as usize)
        }
    }

    /// Clear all bits
    #[inline(always)]
    pub fn clear(&mut self) {
        self.bits = 0;
    }

    /// Check if the bitset is empty
    #[inline(always)]
    pub fn is_clear(&self) -> bool {
        self.bits == 0
    }

    /// Get the capacity (always 64)
    #[inline(always)]
    pub const fn len(&self) -> usize {
        64
    }

    /// Check if empty (alias for is_clear)
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.is_clear()
    }

    /// Grow the bitset (no-op, always 64 bits)
    ///
    /// This exists for API compatibility with FixedBitSet
    #[inline(always)]
    pub fn grow(&mut self, _new_size: usize) {
        // No-op: TinyBitSet is always 64 bits
    }
}

impl fmt::Debug for TinyBitSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TinyBitSet({:#018x})", self.bits)
    }
}

/// Operation type
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Operation {
    // === 기존 (배타적 락 계열) ===
    /// 배타적 락 획득 (Mutex::lock, RwLock::write)
    Request,
    /// 락 해제 (MutexGuard::drop, RwLockWriteGuard::drop)
    Release,

    // === 신규 (공유 접근 계열) ===
    /// 공유 락 획득 (RwLock::read)
    SharedRequest,
    /// 공유 락 해제 (RwLockReadGuard::drop)
    SharedRelease,

    // === 신규 (메모리 계열 — Atomic + Channel) ===
    /// 공유 읽기 (atomic load, channel recv)
    Read,
    /// 배타 쓰기 (atomic store, channel send)
    Write,
    /// 읽기-수정-쓰기 (CAS, fetch_add, fetch_sub)
    ReadWrite,
}

/// Step record in the execution trace
///
/// # TLA+ Correspondence
///
/// ```tla
/// StepRecord == [
///     thread: Threads,
///     op: Operations,
///     resource: Resources,
///     depth: Nat,
///     clock: [Threads -> Nat]
/// ]
/// ```
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct StepRecord {
    /// Thread that executed this step
    pub thread: ThreadId,

    /// Operation performed
    pub operation: Operation,

    /// Resource accessed
    pub resource: ResourceId,

    /// Depth in exploration tree
    pub depth: usize,

    /// Vector clock snapshot at this step
    pub clock: VectorClock,
}

impl fmt::Debug for StepRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Step(t{} {:?} r{} @{})",
            self.thread.as_usize(),
            self.operation,
            self.resource.as_usize(),
            self.depth
        )
    }
}

/// Statistics for DPOR exploration
#[derive(Debug, Clone, Copy, Default)]
pub struct DporStats {
    /// Number of states explored
    pub explored_states: usize,

    /// Number of states pruned
    pub pruned_states: usize,

    /// Number of backtracks performed
    pub backtracks: usize,

    /// Maximum depth reached
    pub max_depth: usize,
}

/// Classic DPOR Scheduler
///
/// # TLA+ Correspondence
///
/// ```tla
/// VARIABLES
///     stack,            -> DporScheduler::stack
///     backtrack_sets,   -> DporScheduler::backtrack_sets
///     done_sets,        -> DporScheduler::done_sets
///     clock_vectors     -> DporScheduler::clock_vectors
/// ```
///
/// # Algorithm
///
/// 1. Start with all threads in backtrack_set[0]
/// 2. Pick a thread from backtrack_set[depth] \ done_set[depth]
/// 3. Execute the thread's operation
/// 4. Scan backwards to find dependent operations
/// 5. For each dependent operation, add current thread to backtrack set
/// 6. Repeat until exploration complete
///
/// # Memory Layout
///
/// ```text
/// stack:          Vec<StepRecord>     (~48 bytes header + depth * 40 bytes)
/// backtrack_sets: Vec<TinyBitSet>     (~24 bytes header + depth * 8 bytes)
/// done_sets:      Vec<TinyBitSet>     (~24 bytes header + depth * 8 bytes)
/// clock_vectors:  Vec<VectorClock>    (~24 bytes header + threads * 64 bytes)
/// Total overhead: ~120 bytes + exploration data
/// ```
pub struct DporScheduler {
    /// Execution trace stack
    ///
    /// TLA+: `stack: Seq(StepRecord)`
    pub(crate) stack: Vec<StepRecord>,

    /// Threads to explore at each depth
    ///
    /// TLA+: `backtrack_sets: [Depth -> SUBSET Threads]`
    pub(crate) backtrack_sets: Vec<TinyBitSet>,

    /// Threads already explored at each depth
    ///
    /// TLA+: `done_sets: [Depth -> SUBSET Threads]`
    pub(crate) done_sets: Vec<TinyBitSet>,

    /// Vector clocks for each thread
    ///
    /// TLA+: `clock_vectors: [Thread -> [Thread -> Nat]]`
    pub(crate) clock_vectors: Vec<VectorClock>,

    /// Number of threads
    num_threads: usize,

    /// Statistics
    stats: DporStats,
}

impl DporScheduler {
    /// Create a new DPOR scheduler
    ///
    /// # Arguments
    ///
    /// * `num_threads` - Number of threads to schedule (must be <= 64)
    ///
    /// # Panics
    ///
    /// Panics if `num_threads > MAX_THREADS` or `num_threads > 64`
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// DporInit ==
    ///     /\ stack = <<>>
    ///     /\ current_depth = 0
    ///     /\ backtrack_sets = [d \in 0..MaxDepth |->
    ///                             IF d = 0 THEN Threads ELSE {}]
    ///     /\ done_sets = [d \in 0..MaxDepth |-> {}]
    ///     /\ clock_vectors = [t1 \in Threads |-> [t2 \in Threads |-> 0]]
    /// ```
    pub fn new(num_threads: usize) -> Self {
        assert!(
            num_threads <= MAX_THREADS,
            "Too many threads: {}",
            num_threads
        );
        assert!(num_threads <= 64, "TinyBitSet supports max 64 threads");

        // Initialize backtrack_sets with first depth containing all threads
        let mut backtrack_sets = Vec::with_capacity(MAX_DEPTH);
        let mut initial_set = TinyBitSet::new(num_threads);
        for i in 0..num_threads {
            initial_set.insert(i);
        }
        backtrack_sets.push(initial_set);

        // Initialize done_sets (all empty initially)
        let mut done_sets = Vec::with_capacity(MAX_DEPTH);
        done_sets.push(TinyBitSet::new(num_threads));

        // Initialize vector clocks
        let clock_vectors = vec![VectorClock::new(); num_threads];

        Self {
            stack: Vec::with_capacity(MAX_DEPTH),
            backtrack_sets,
            done_sets,
            clock_vectors,
            num_threads,
            stats: DporStats::default(),
        }
    }

    /// Get the current depth in the exploration tree
    #[inline]
    pub fn current_depth(&self) -> usize {
        self.stack.len()
    }

    /// Get the next thread to execute
    ///
    /// Returns `Some(thread_id)` if there's a thread to explore at current depth,
    /// or `None` if backtracking is needed.
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// LET available == backtrack_sets[current_depth] \ done_sets[current_depth]
    /// IN available # {}
    ///    /\ \E t \in available : ...
    /// ```
    pub fn next_step(&mut self) -> Option<ThreadId> {
        let depth = self.current_depth();

        // Ensure we have backtrack/done sets for this depth
        while self.backtrack_sets.len() <= depth {
            self.backtrack_sets.push(TinyBitSet::new(self.num_threads));
            self.done_sets.push(TinyBitSet::new(self.num_threads));
        }

        // Compute: backtrack_set \ done_set
        // TinyBitSet is Copy, so this is a register copy (not heap clone!)
        let mut available = self.backtrack_sets[depth];
        available.difference_with(&self.done_sets[depth]);

        // Pick first available thread using CTZ instruction
        available.next_set_bit().map(ThreadId)
    }

    /// Commit a step after execution
    ///
    /// This is THE CORE of DPOR: we scan backwards through the stack to find
    /// dependent operations and add backtrack points.
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// UpdateBacktrackSets(current_step) ==
    ///     [d \in 0..MaxDepth |->
    ///         IF d < Len(stack) THEN
    ///             LET past_step == stack[d + 1]
    ///             IN IF NeedBacktrack(past_step, current_step, d)
    ///                THEN backtrack_sets[d] \cup {current_step.thread}
    ///                ELSE backtrack_sets[d]
    ///         ELSE backtrack_sets[d]
    ///     ]
    /// ```
    pub fn commit_step(&mut self, thread: ThreadId, operation: Operation, resource: ResourceId) {
        let depth = self.current_depth();

        // Ensure we have sets for current depth
        while self.backtrack_sets.len() <= depth {
            self.backtrack_sets.push(TinyBitSet::new(self.num_threads));
        }
        while self.done_sets.len() <= depth {
            self.done_sets.push(TinyBitSet::new(self.num_threads));
        }

        // Create step record with current vector clock
        let current_clock = self.clock_vectors[thread.as_usize()];
        let step = StepRecord {
            thread,
            operation,
            resource,
            depth,
            clock: current_clock,
        };

        // DPOR Magic: Scan backwards to find dependent operations
        for i in 0..self.stack.len() {
            let past_step = &self.stack[i];

            if self.is_dependent(&step, past_step) && self.is_concurrent(&step, past_step) {
                // Add current thread to backtrack set at past step's depth
                let past_depth = past_step.depth;

                // Ensure backtrack_sets is large enough
                while self.backtrack_sets.len() <= past_depth {
                    self.backtrack_sets.push(TinyBitSet::new(self.num_threads));
                }

                self.backtrack_sets[past_depth].insert(thread.as_usize());
            }
        }

        // Update vector clock for this thread
        self.clock_vectors[thread.as_usize()].tick(thread);

        // Mark thread as done at current depth
        self.done_sets[depth].insert(thread.as_usize());

        // Push step onto stack
        self.stack.push(step);

        // Initialize NEXT depth (critical for AB-BA deadlock detection!)
        // We are entering a new node in the state space.
        // We must ensure the sets for next_depth are clean and initialized.
        let next_depth = depth + 1;
        if next_depth < MAX_DEPTH {
            // Ensure capacity
            while self.backtrack_sets.len() <= next_depth {
                self.backtrack_sets.push(TinyBitSet::new(self.num_threads));
            }
            while self.done_sets.len() <= next_depth {
                self.done_sets.push(TinyBitSet::new(self.num_threads));
            }

            // CLEAR stale data from previous explorations at this depth
            self.backtrack_sets[next_depth].clear();
            self.done_sets[next_depth].clear();

            // Add all threads to next depth's backtrack set (Conservative DPOR)
            for t in 0..self.num_threads {
                self.backtrack_sets[next_depth].insert(t);
            }
        }

        // Update stats
        self.stats.explored_states += 1;
        self.stats.max_depth = self.stats.max_depth.max(depth + 1);
    }

    /// Backtrack to previous depth
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// Backtrack ==
    ///     /\ current_depth > 0
    ///     /\ backtrack_sets[current_depth] \ done_sets[current_depth] = {}
    ///     /\ stack' = SubSeq(stack, 1, Len(stack) - 1)
    ///     /\ current_depth' = current_depth - 1
    /// ```
    pub fn backtrack(&mut self) -> bool {
        if self.stack.is_empty() {
            return false; // Exploration complete
        }

        // Pop the last step
        let popped_step = self.stack.pop().unwrap();

        // Restore vector clock to state before this step
        // The StepRecord contains the clock snapshot from BEFORE tick()
        self.clock_vectors[popped_step.thread.as_usize()] = popped_step.clock;

        // Update stats
        self.stats.backtracks += 1;

        true
    }

    /// Check if two steps are dependent
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// IsDependentOp(step1, step2) ==
    ///     /\ step1.resource = step2.resource
    ///     /\ step1.thread # step2.thread
    ///     /\ \/ step1.op = "Request" /\ step2.op = "Request"
    ///        \/ step1.op = "Request" /\ step2.op = "Release"
    ///        \/ step1.op = "Release" /\ step2.op = "Request"
    ///        \/ step1.op = "Release" /\ step2.op = "Release"
    /// ```
    #[inline]
    pub(crate) fn is_dependent(&self, step1: &StepRecord, step2: &StepRecord) -> bool {
        if step1.resource != step2.resource || step1.thread == step2.thread {
            return false;
        }
        // 충돌 매트릭스 — 비충돌 조합만 명시적 제외, 나머지 모두 충돌
        // SharedRequest+SharedRequest, SharedRelease+SharedRelease = 비충돌
        // SharedRequest+SharedRelease, SharedRelease+SharedRequest = 비충돌
        // Read+Read = 비충돌
        // SharedRequest+Read, Read+SharedRequest = 비충돌
        // SharedRelease+Read, Read+SharedRelease = 비충돌
        !matches!(
            (step1.operation, step2.operation),
            // 공유 락 계열끼리 (읽기 = 비충돌)
            (Operation::SharedRequest, Operation::SharedRequest)
                | (Operation::SharedRequest, Operation::SharedRelease)
                | (Operation::SharedRelease, Operation::SharedRequest)
                | (Operation::SharedRelease, Operation::SharedRelease)
            // 메모리 읽기끼리 (atomic load 동시 접근 = 비충돌)
                | (Operation::Read, Operation::Read)
            // 공유 락 + 메모리 읽기 교차 (모두 읽기 계열 = 비충돌)
                | (Operation::SharedRequest, Operation::Read)
                | (Operation::Read, Operation::SharedRequest)
                | (Operation::SharedRelease, Operation::Read)
                | (Operation::Read, Operation::SharedRelease)
        )
    }

    /// Check if two steps are concurrent (neither happens-before the other)
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// IsConcurrent(step1, step2) ==
    ///     /\ ~HappensBefore(step1, step2)
    ///     /\ ~HappensBefore(step2, step1)
    /// ```
    #[inline]
    pub(crate) fn is_concurrent(&self, step1: &StepRecord, step2: &StepRecord) -> bool {
        step1.clock.concurrent(&step2.clock)
    }

    /// Check if exploration is complete
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// ExplorationComplete ==
    ///     /\ current_depth = 0
    ///     /\ backtrack_sets[0] \ done_sets[0] = {}
    /// ```
    pub fn is_complete(&self) -> bool {
        self.stack.is_empty() && {
            if self.backtrack_sets.is_empty() || self.done_sets.is_empty() {
                true
            } else {
                let mut available = self.backtrack_sets[0];
                available.difference_with(&self.done_sets[0]);
                available.is_clear()
            }
        }
    }

    /// Get exploration statistics
    pub fn stats(&self) -> DporStats {
        self.stats
    }

    /// Get current stack (for debugging/testing)
    pub fn stack(&self) -> &[StepRecord] {
        &self.stack
    }

    /// Extract the current execution path as a [`Schedule`].
    ///
    /// Returns `Some` when the stack is non-empty (i.e., there is an active
    /// exploration path to capture). Classic DPOR does not detect liveness
    /// violations, so `violation` is always `None`.
    ///
    /// Returns `None` when the scheduler is at depth 0 with an empty stack.
    pub fn extract_schedule(&self) -> Option<crate::dpor::schedule::Schedule> {
        if self.stack.is_empty() {
            return None;
        }
        Some(crate::dpor::schedule::Schedule {
            steps: self.stack.clone(),
            violation: None,
        })
    }

    /// Reset the scheduler to initial state
    pub fn reset(&mut self) {
        self.stack.clear();
        self.backtrack_sets.clear();
        self.done_sets.clear();

        // Re-initialize first depth
        let mut initial_set = TinyBitSet::new(self.num_threads);
        for i in 0..self.num_threads {
            initial_set.insert(i);
        }
        self.backtrack_sets.push(initial_set);
        self.done_sets.push(TinyBitSet::new(self.num_threads));

        // Reset clocks
        for clock in &mut self.clock_vectors {
            *clock = VectorClock::new();
        }

        self.stats = DporStats::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tiny_bitset_basic() {
        let mut bs = TinyBitSet::new(8);

        assert!(bs.is_clear());
        assert!(!bs.contains(0));

        bs.insert(0);
        assert!(bs.contains(0));
        assert!(!bs.is_clear());

        bs.insert(3);
        assert!(bs.contains(3));

        bs.clear();
        assert!(bs.is_clear());
    }

    #[test]
    fn test_tiny_bitset_difference() {
        let mut a = TinyBitSet::new(8);
        let mut b = TinyBitSet::new(8);

        a.insert(0);
        a.insert(1);
        a.insert(2);

        b.insert(1);
        b.insert(2);

        a.difference_with(&b);

        assert!(a.contains(0));
        assert!(!a.contains(1));
        assert!(!a.contains(2));
    }

    #[test]
    fn test_tiny_bitset_next_set_bit() {
        let mut bs = TinyBitSet::new(8);

        assert_eq!(bs.next_set_bit(), None);

        bs.insert(3);
        assert_eq!(bs.next_set_bit(), Some(3));

        bs.insert(0);
        assert_eq!(bs.next_set_bit(), Some(0));
    }

    #[test]
    fn test_scheduler_init() {
        let scheduler = DporScheduler::new(3);
        assert_eq!(scheduler.current_depth(), 0);
        assert_eq!(scheduler.num_threads, 3);
    }

    #[test]
    fn test_next_step() {
        let mut scheduler = DporScheduler::new(2);
        let thread = scheduler.next_step();
        assert!(thread.is_some());
    }

    #[test]
    fn test_commit_step() {
        let mut scheduler = DporScheduler::new(2);

        scheduler.commit_step(ThreadId(0), Operation::Request, ResourceId(0));

        assert_eq!(scheduler.current_depth(), 1);
        assert_eq!(scheduler.stats().explored_states, 1);
    }

    #[test]
    fn test_backtrack() {
        let mut scheduler = DporScheduler::new(2);

        scheduler.commit_step(ThreadId(0), Operation::Request, ResourceId(0));
        assert_eq!(scheduler.current_depth(), 1);

        scheduler.backtrack();
        assert_eq!(scheduler.current_depth(), 0);
        assert_eq!(scheduler.stats().backtracks, 1);
    }

    #[test]
    fn test_dependency_detection() {
        let scheduler = DporScheduler::new(2);

        let step1 = StepRecord {
            thread: ThreadId(0),
            operation: Operation::Request,
            resource: ResourceId(0),
            depth: 0,
            clock: VectorClock::new(),
        };

        let step2 = StepRecord {
            thread: ThreadId(1),
            operation: Operation::Request,
            resource: ResourceId(0),
            depth: 1,
            clock: VectorClock::new(),
        };

        assert!(scheduler.is_dependent(&step1, &step2));

        let step3 = StepRecord {
            thread: ThreadId(1),
            operation: Operation::Request,
            resource: ResourceId(1),
            depth: 1,
            clock: VectorClock::new(),
        };

        assert!(!scheduler.is_dependent(&step1, &step3));
    }

    #[test]
    fn test_tiny_bitset_copy_semantics() {
        let mut a = TinyBitSet::new(8);
        a.insert(0);
        a.insert(1);

        let b = a;

        assert!(b.contains(0));
        assert!(b.contains(1));

        a.clear();

        assert!(b.contains(0));
        assert!(b.contains(1));
    }
}
