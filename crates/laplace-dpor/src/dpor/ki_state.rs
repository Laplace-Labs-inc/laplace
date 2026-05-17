// SPDX-License-Identifier: Apache-2.0
//! Ki-DPOR State Node (A* Node) with Liveness Tracking
//!
//! This module implements the ExecutionState from the TLA+ specification,
//! extended with fairness tracking for starvation detection.

use super::classic::{Operation, StepRecord, TinyBitSet};
use super::vector_clock::VectorClock;
// MAX_THREADS is a physical array-size bound (must be a const); the configurable
// logical limit is stored per-state as `self.max_threads`.
use super::MAX_THREADS;
use laplace_interfaces::{
    domain::resource::{ResourceId, ThreadId},
    AxiomConfig,
};
use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Thread status enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThreadStatus {
    /// Thread is running
    Running,
    /// Thread is blocked on a resource
    Blocked,
}

/// Ki-DPOR State (ExecutionState in TLA+) with Liveness
///
/// # TLA+ Correspondence
///
/// ```tla
/// LivenessExecutionState == [
///     path: Seq(StepRecord),
///     cost_g: Nat,
///     heuristic_h: Nat,
///     priority_f: Nat,
///     resource_state: ...,
///     starvation_counters_state: [Threads -> Nat],  # NEW
///     fairness_score: Nat                            # NEW
/// ]
/// ```
///
/// # Liveness Properties
///
/// - `starvation_counters`: Steps since each thread made progress
/// - If counter exceeds MAX_STARVATION_LIMIT, we found a starvation bug
#[derive(Clone)]
pub struct KiState {
    /// Execution path (sequence of steps)
    pub path: Vec<StepRecord>,

    /// Actual cost from start (g in A*)
    pub cost_g: usize,

    /// Heuristic estimate (h in A*)
    pub heuristic_h: usize,

    /// Total priority (f = g + h in A*)
    pub priority_f: usize,

    // ========== State Snapshots ==========
    // A* requires full state restoration since we jump between branches
    /// Resource ownership snapshot
    /// None = free, Some(thread) = owned by thread
    pub resource_owners: Vec<Option<ThreadId>>,

    /// Waiting queues snapshot
    /// waiting_queues[r] = [t1, t2, ...] threads waiting for resource r
    pub waiting_queues: Vec<Vec<ThreadId>>,

    /// Thread status snapshot
    pub thread_status: Vec<ThreadStatus>,

    /// Vector clock snapshot
    pub clock_vectors: Vec<VectorClock>,

    // ========== Liveness Tracking ==========
    /// Starvation counters: steps since each thread executed
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// starvation_counters_state: [Threads -> Nat]
    /// ```
    ///
    /// Updated by: UpdateStarvationCounters(state, executed_thread)
    pub starvation_counters: Vec<usize>,

    /// Cached hash for state signature
    state_hash: u64,

    /// Maximum number of threads (injected from config)
    max_threads: usize,

    /// Maximum danger score ceiling for heuristic (injected from config)
    max_danger: usize,
}

impl KiState {
    /// Create initial state
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// initial_state == [
    ///     path |-> <<>>,
    ///     cost_g |-> 0,
    ///     heuristic_h |-> Heuristic(initial),
    ///     priority_f |-> 0,
    ///     ...
    /// ]
    /// ```
    pub fn initial(num_threads: usize, num_resources: usize) -> Self {
        Self::initial_with_config(
            num_threads,
            num_resources,
            num_threads,
            AxiomConfig::default().max_danger as usize,
        )
    }

    /// Create initial state with injected config values.
    pub fn initial_with_config(
        num_threads: usize,
        num_resources: usize,
        max_threads: usize,
        max_danger: usize,
    ) -> Self {
        let resource_owners = vec![None; num_resources];
        let waiting_queues = vec![Vec::new(); num_resources];
        let thread_status = vec![ThreadStatus::Running; num_threads];
        let clock_vectors = vec![VectorClock::new(); num_threads];
        let starvation_counters = vec![0; num_threads]; // All start at 0

        let state = Self {
            path: Vec::new(),
            cost_g: 0,
            heuristic_h: 0,
            priority_f: 0,
            resource_owners,
            waiting_queues,
            thread_status,
            clock_vectors,
            starvation_counters,
            state_hash: 0,
            max_threads,
            max_danger,
        };

        let mut state = state;
        state.recompute_heuristic();
        state.recompute_hash();
        state
    }

    /// Generate successor state
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// successor(curr, thread, op, res) == [
    ///     path |-> Append(curr.path, [thread |-> thread, op |-> op, res |-> res]),
    ///     cost_g |-> curr.cost_g + 1,
    ///     ...
    /// ]
    /// ```
    pub fn successor(&self, thread: ThreadId, operation: Operation, resource: ResourceId) -> Self {
        let mut new_state = self.clone();

        // Add step to path
        let step = StepRecord {
            thread,
            operation,
            resource,
            depth: self.path.len(),
            clock: self.clock_vectors[thread.as_usize()],
        };
        new_state.path.push(step);

        // Update cost
        new_state.cost_g = self.cost_g + 1;

        // Apply operation to snapshot
        new_state.apply_operation(thread, operation, resource);

        // Update starvation counters (CRITICAL FOR LIVENESS)
        new_state.update_starvation_counters(thread);

        // Recompute heuristic and priority
        new_state.recompute_heuristic();
        new_state.priority_f = new_state.cost_g + new_state.heuristic_h;

        // Recompute hash
        new_state.recompute_hash();

        new_state
    }

    /// Apply an operation to the state snapshot
    fn apply_operation(&mut self, thread: ThreadId, operation: Operation, resource: ResourceId) {
        let r = resource.as_usize();
        let t = thread.as_usize();

        match operation {
            Operation::Request => {
                if self.resource_owners[r].is_none() {
                    // Resource is free, acquire it
                    self.resource_owners[r] = Some(thread);
                    // Remove from waiting queue if present
                    self.waiting_queues[r].retain(|&waiting_thread| waiting_thread != thread);
                    // Fix A: restore Running status (may have been Blocked on prior attempt)
                    self.thread_status[t] = ThreadStatus::Running;
                } else {
                    // Resource is held, block and add to queue (if not already there)
                    self.thread_status[t] = ThreadStatus::Blocked;
                    if !self.waiting_queues[r].contains(&thread) {
                        self.waiting_queues[r].push(thread);
                    }
                }
            }
            Operation::Release => {
                // Release resource (make it free)
                if self.resource_owners[r] == Some(thread) {
                    self.resource_owners[r] = None;

                    // DO NOT automatically wake up waiters
                    // Let them compete in next Request operations
                    // This allows unfair scheduling and starvation scenarios
                }
            }
            // SharedRequest: 공유 락 획득 (읽기 가능, 여러 스레드 동시 소유)
            Operation::SharedRequest => {
                // For simplicity, treat shared read lock acquisition similar to exclusive
                // The real concurrency model tracks read vs write separately in the DPOR engine
                if self.resource_owners[r].is_none() {
                    self.resource_owners[r] = Some(thread);
                    self.waiting_queues[r].retain(|&waiting_thread| waiting_thread != thread);
                    // Fix A: restore Running status after successful acquire
                    self.thread_status[t] = ThreadStatus::Running;
                } else {
                    self.thread_status[t] = ThreadStatus::Blocked;
                    if !self.waiting_queues[r].contains(&thread) {
                        self.waiting_queues[r].push(thread);
                    }
                }
            }
            // SharedRelease: 공유 락 해제
            Operation::SharedRelease => {
                if self.resource_owners[r] == Some(thread) {
                    self.resource_owners[r] = None;
                }
            }
            // Read: 공유 읽기 (atomic load, channel recv)
            Operation::Read => {
                // Read operations don't block; they're always allowed
                // No state change needed
            }
            // Write: 배타 쓰기 (atomic store, channel send)
            Operation::Write => {
                if self.resource_owners[r].is_none() {
                    self.resource_owners[r] = Some(thread);
                    self.waiting_queues[r].retain(|&waiting_thread| waiting_thread != thread);
                    // Fix A: restore Running status after successful acquire
                    self.thread_status[t] = ThreadStatus::Running;
                } else {
                    self.thread_status[t] = ThreadStatus::Blocked;
                    if !self.waiting_queues[r].contains(&thread) {
                        self.waiting_queues[r].push(thread);
                    }
                }
            }
            // ReadWrite: 읽기-수정-쓰기 (CAS, fetch_add, fetch_sub)
            Operation::ReadWrite => {
                if self.resource_owners[r].is_none() {
                    self.resource_owners[r] = Some(thread);
                    self.waiting_queues[r].retain(|&waiting_thread| waiting_thread != thread);
                    // Fix A: restore Running status after successful acquire
                    self.thread_status[t] = ThreadStatus::Running;
                } else {
                    self.thread_status[t] = ThreadStatus::Blocked;
                    if !self.waiting_queues[r].contains(&thread) {
                        self.waiting_queues[r].push(thread);
                    }
                }
            }
        }

        // Update vector clock
        self.clock_vectors[t].tick(thread);
    }

    /// Update starvation counters after a step
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// UpdateStarvationCounters(state, executed_thread) ==
    ///     [t \in Threads |->
    ///         IF t = executed_thread
    ///         THEN 0  \* Reset counter
    ///         ELSE IF state.thread_status_state[t] \in {"Running", "Blocked"}
    ///              THEN state.starvation_counters_state[t] + 1
    ///              ELSE state.starvation_counters_state[t]
    ///     ]
    /// ```
    fn update_starvation_counters(&mut self, executed_thread: ThreadId) {
        let t_exec = executed_thread.as_usize();
        // A thread "made progress" only if it is Running after the operation
        // (i.e. it successfully acquired a resource or completed a release).
        // A thread that is still Blocked after executing did NOT make progress —
        // its operation was a no-op and should not reset the starvation counter.
        let made_progress = self.thread_status[t_exec] == ThreadStatus::Running;

        for (t_idx, counter) in self.starvation_counters.iter_mut().enumerate() {
            if t_idx == t_exec {
                if made_progress {
                    // Thread acquired or released — genuine progress, reset.
                    *counter = 0;
                } else {
                    // Thread is still Blocked after executing (no-op operation).
                    // Treat as not making progress: continue incrementing.
                    *counter += 1;
                }
            } else {
                // Increment counter for threads that didn't execute
                // (but only if they're Running or Blocked)
                let status = self.thread_status[t_idx];
                if status == ThreadStatus::Running || status == ThreadStatus::Blocked {
                    *counter += 1;
                }
            }
        }
    }

    /// Compute heuristic value with fairness penalty
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// LivenessHeuristic(state) ==
    ///     LET danger_score == (blocked * 100) + (contention * 10) + (interleaving * 5)
    ///         total_starvation == TotalStarvation(state)
    ///         max_starvation == MaxStarvation(state)
    ///         fairness_score == (total_starvation * 50) + (max_starvation * 20)
    ///         combined_danger == danger_score + fairness_score
    ///         max_danger == 2000
    ///     IN max_danger - combined_danger
    /// ```
    ///
    /// # Philosophy
    ///
    /// To FIND starvation bugs, we must actively search for unfair paths.
    /// High starvation = LOWER h = HIGHER priority = Explored sooner.
    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(
            layer = "30_Axiom_DPOR",
            link = "LEP-0009-laplace-axiom-dpor_heuristic_and_ghosting"
        )
    )]
    fn recompute_heuristic(&mut self) {
        // Original danger metrics
        let blocked_count = self.blocked_threads_count();
        let contention_score = self.contention_score();
        let interleaving_complexity = self.interleaving_complexity();

        let danger_score =
            (blocked_count * 100) + (contention_score * 10) + (interleaving_complexity * 5);

        // Fairness metrics (NEW)
        let total_starvation = self.total_starvation();
        let max_starvation = self.max_starvation();

        let fairness_score = (total_starvation * 50) + (max_starvation * 20);

        // Combined score
        let combined_danger = danger_score + fairness_score;

        // Invert so lower h = higher priority
        self.heuristic_h = self.max_danger.saturating_sub(combined_danger);
    }

    /// Count blocked threads
    #[inline]
    fn blocked_threads_count(&self) -> usize {
        self.thread_status
            .iter()
            .filter(|&&status| status == ThreadStatus::Blocked)
            .count()
    }

    /// Calculate contention score (sum of waiting queue lengths)
    #[inline]
    fn contention_score(&self) -> usize {
        self.waiting_queues.iter().map(|q| q.len()).sum()
    }

    /// Calculate interleaving complexity (distinct threads in path)
    #[inline]
    fn interleaving_complexity(&self) -> usize {
        let max_threads = self.max_threads;
        let mut threads = TinyBitSet::new(max_threads);
        for step in &self.path {
            threads.insert(step.thread.as_usize());
        }

        (0..max_threads).filter(|&i| threads.contains(i)).count()
    }

    /// Total starvation (sum of all counters)
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// TotalStarvation(state) ==
    ///     LET counters == {state.starvation_counters_state[t] : t \in Threads}
    ///     IN SumSeq(SetToSeq(counters))
    /// ```
    #[inline]
    fn total_starvation(&self) -> usize {
        self.starvation_counters.iter().sum()
    }

    /// Maximum starvation (highest counter)
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// MaxStarvation(state) ==
    ///     LET counters == {state.starvation_counters_state[t] : t \in Threads}
    ///     IN CHOOSE max \in Nat : max \in counters /\ \A c \in counters : c <= max
    /// ```
    #[inline]
    fn max_starvation(&self) -> usize {
        *self.starvation_counters.iter().max().unwrap_or(&0)
    }

    /// Detect a cycle in the Wait-For Graph (WFG) derived from current resource state.
    ///
    /// # Wait-For Graph
    ///
    /// An edge `T_a → T_b` exists iff `T_a` is in `waiting_queues[r]` for some
    /// resource `r` where `resource_owners[r] == Some(T_b)`.  This graph is
    /// built on-demand from the snapshots already stored in `KiState`, so no
    /// extra persistent field is required.
    ///
    /// # Returns
    ///
    /// `Some(cycle)` — ordered list of `ThreadId`s forming the first detected
    /// circular-wait chain (partial deadlock), or `None` if the WFG is acyclic.
    ///
    /// # Optimization
    ///
    /// Should only be called when at least one thread is `Blocked`.  An
    /// all-`Running` state has no WFG edges and therefore no cycle.
    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(
            layer = "30_Axiom_DPOR",
            link = "LEP-0009-laplace-axiom-dpor_heuristic_and_ghosting"
        )
    )]
    pub fn detect_wfg_cycle(&self) -> Option<Vec<ThreadId>> {
        let n = self.thread_status.len();

        // Build adjacency matrix: wfg[waiter][owner] = true.
        // Fix B: only include waiters that are actually Blocked.
        // Stale waiting_queues entries must not become WFG edges.
        let mut wfg = [[false; MAX_THREADS]; MAX_THREADS];
        for r in 0..self.resource_owners.len() {
            if let Some(owner) = self.resource_owners[r] {
                for &waiter in &self.waiting_queues[r] {
                    if self.thread_status[waiter.as_usize()] == ThreadStatus::Blocked {
                        wfg[waiter.as_usize()][owner.as_usize()] = true;
                    }
                }
            }
        }

        // DFS cycle detection — "recursion-stack" (on_stack) variant.
        let mut visited = [false; MAX_THREADS];
        let mut on_stack = [false; MAX_THREADS];
        let mut path: Vec<usize> = Vec::with_capacity(n);

        for start in 0..n {
            if !visited[start] {
                if let Some(cycle) =
                    Self::wfg_dfs(start, &wfg, &mut visited, &mut on_stack, &mut path, n)
                {
                    return Some(cycle);
                }
            }
        }

        None
    }

    /// DFS helper used by [`detect_wfg_cycle`].
    ///
    /// Returns `Some(cycle)` when a back-edge is found (cycle reconstructed
    /// from the current DFS path), or `None` if the sub-tree is acyclic.
    fn wfg_dfs(
        node: usize,
        wfg: &[[bool; MAX_THREADS]; MAX_THREADS],
        visited: &mut [bool; MAX_THREADS],
        on_stack: &mut [bool; MAX_THREADS],
        path: &mut Vec<usize>,
        n: usize,
    ) -> Option<Vec<ThreadId>> {
        visited[node] = true;
        on_stack[node] = true;
        path.push(node);

        for next in 0..n {
            if wfg[node][next] {
                if on_stack[next] {
                    // Back-edge found: cycle starts at `next` in the current path.
                    let cycle_start = path.iter().position(|&x| x == next).unwrap();
                    return Some(path[cycle_start..].iter().map(|&t| ThreadId(t)).collect());
                }
                if !visited[next] {
                    if let Some(cycle) = Self::wfg_dfs(next, wfg, visited, on_stack, path, n) {
                        return Some(cycle);
                    }
                }
            }
        }

        on_stack[node] = false;
        path.pop();
        None
    }

    /// Compute state signature hash
    fn recompute_hash(&mut self) {
        let mut hasher = DefaultHasher::new();

        for owner in &self.resource_owners {
            owner.hash(&mut hasher);
        }

        for status in &self.thread_status {
            status.hash(&mut hasher);
        }

        for queue in &self.waiting_queues {
            queue.hash(&mut hasher);
        }

        // Include starvation counters so that states differing only in how long
        // threads have been waiting are treated as distinct.  Without this, the
        // DPOR's explored-set deduplication collapses all starvation-accumulating
        // states onto the first visit and exploration terminates prematurely.
        for &c in &self.starvation_counters {
            c.hash(&mut hasher);
        }

        self.state_hash = hasher.finish();
    }

    /// Get state signature hash
    pub fn signature(&self) -> u64 {
        self.state_hash
    }

    /// Get enabled (runnable) threads
    ///
    /// Returns threads that can make progress:
    /// - Running threads (always enabled)
    /// - Blocked threads (may try Request again when resource becomes free)
    pub fn enabled_threads(&self) -> Vec<ThreadId> {
        self.thread_status
            .iter()
            .enumerate()
            .map(|(i, &status)| match status {
                ThreadStatus::Running => ThreadId(i),
                ThreadStatus::Blocked => {
                    // Blocked threads are still "enabled" in the sense that
                    // they can attempt operations (retry Request)
                    ThreadId(i)
                }
            })
            .collect()
    }
}

// ============================================================================
// Ordering for BinaryHeap (Min-Heap behavior)
// ============================================================================

impl Ord for KiState {
    fn cmp(&self, other: &Self) -> Ordering {
        // REVERSE: Lower priority_f = Greater ordering = Popped first
        other.priority_f.cmp(&self.priority_f)
    }
}

impl PartialOrd for KiState {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for KiState {
    fn eq(&self, other: &Self) -> bool {
        self.priority_f == other.priority_f
    }
}

impl Eq for KiState {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state_liveness() {
        let state = KiState::initial(2, 2);

        assert_eq!(state.starvation_counters.len(), 2);
        assert!(state.starvation_counters.iter().all(|&c| c == 0));
    }

    #[test]
    fn test_starvation_counter_update() {
        let mut state = KiState::initial(3, 2);

        // Thread 0 executes
        state.update_starvation_counters(ThreadId(0));

        // Thread 0 should be reset to 0
        assert_eq!(state.starvation_counters[0], 0);
        // Thread 1 and 2 should increment (they're Running but didn't execute)
        assert_eq!(state.starvation_counters[1], 1);
        assert_eq!(state.starvation_counters[2], 1);

        // Thread 0 executes again
        state.update_starvation_counters(ThreadId(0));

        assert_eq!(state.starvation_counters[0], 0);
        assert_eq!(state.starvation_counters[1], 2); // Keeps incrementing
        assert_eq!(state.starvation_counters[2], 2);
    }

    #[test]
    fn test_heuristic_with_starvation() {
        let mut state = KiState::initial(3, 2);

        let initial_h = state.heuristic_h;

        // Simulate starvation
        state.starvation_counters[1] = 5;
        state.starvation_counters[2] = 3;
        state.recompute_heuristic();

        // h should decrease (higher danger = lower h = higher priority)
        assert!(state.heuristic_h < initial_h);
    }

    #[test]
    fn test_fairness_metrics() {
        let mut state = KiState::initial(3, 2);

        state.starvation_counters[0] = 5;
        state.starvation_counters[1] = 3;
        state.starvation_counters[2] = 2;

        assert_eq!(state.total_starvation(), 10);
        assert_eq!(state.max_starvation(), 5);
    }

    #[test]
    fn test_successor_resets_executing_thread_counter() {
        let state = KiState::initial(2, 2);

        // Create successor where thread 0 executes
        let successor = state.successor(ThreadId(0), Operation::Request, ResourceId(0));

        // Thread 0's counter should be 0
        assert_eq!(successor.starvation_counters[0], 0);
        // Thread 1's counter should be 1 (didn't execute)
        assert_eq!(successor.starvation_counters[1], 1);
    }

    /// Fix A 검증: Blocked 상태에서 자원 획득 시 Running으로 복구되는지
    #[test]
    fn test_blocked_to_running_on_acquire() {
        // Setup: T0 holds R0, T1 becomes Blocked waiting for R0
        let mut state = KiState::initial(2, 1);

        // T0 acquires R0
        state.apply_operation(ThreadId(0), Operation::Request, ResourceId(0));
        assert_eq!(state.resource_owners[0], Some(ThreadId(0)));
        assert_eq!(state.thread_status[0], ThreadStatus::Running);

        // T1 tries R0 → Blocked
        state.apply_operation(ThreadId(1), Operation::Request, ResourceId(0));
        assert_eq!(state.thread_status[1], ThreadStatus::Blocked);
        assert!(state.waiting_queues[0].contains(&ThreadId(1)));

        // T0 releases R0
        state.apply_operation(ThreadId(0), Operation::Release, ResourceId(0));
        assert_eq!(state.resource_owners[0], None);

        // T1 retries R0 → should acquire AND return to Running
        state.apply_operation(ThreadId(1), Operation::Request, ResourceId(0));
        assert_eq!(state.resource_owners[0], Some(ThreadId(1)));
        assert_eq!(state.thread_status[1], ThreadStatus::Running);
        assert!(!state.waiting_queues[0].contains(&ThreadId(1)));
    }

    /// Fix B 검증: stale waiting_queues 엔트리가 WFG 엣지를 생성하지 않는지
    #[test]
    fn test_no_false_wfg_edge_for_stale_waiter() {
        // mio 실험에서 발견된 false positive 시나리오 재현
        // T0: Req(R1) → T1: Req(R1) blocked → T1: Req(R0) → T0: Req(R0) blocked
        let mut state = KiState::initial(2, 2);

        // T0 acquires R1
        state.apply_operation(ThreadId(0), Operation::Request, ResourceId(1));
        assert_eq!(state.resource_owners[1], Some(ThreadId(0)));

        // T1 tries R1 → Blocked, queued
        state.apply_operation(ThreadId(1), Operation::Request, ResourceId(1));
        assert_eq!(state.thread_status[1], ThreadStatus::Blocked);

        // T0 releases R1 (waiting_queues[1] may still contain stale entries)
        state.apply_operation(ThreadId(0), Operation::Release, ResourceId(1));
        assert_eq!(state.resource_owners[1], None);

        // T1 retries R1 → acquires, becomes Running (Fix A)
        state.apply_operation(ThreadId(1), Operation::Request, ResourceId(1));
        assert_eq!(state.resource_owners[1], Some(ThreadId(1)));
        assert_eq!(state.thread_status[1], ThreadStatus::Running);

        // T1 acquires R0
        state.apply_operation(ThreadId(1), Operation::Request, ResourceId(0));
        assert_eq!(state.resource_owners[0], Some(ThreadId(1)));

        // T0 tries R0 → Blocked
        state.apply_operation(ThreadId(0), Operation::Request, ResourceId(0));
        assert_eq!(state.thread_status[0], ThreadStatus::Blocked);

        // WFG should contain only T0→T1 (no cycle)
        let cycle = state.detect_wfg_cycle();
        assert!(
            cycle.is_none(),
            "Should be CLEAN: T1 can release R0 to unblock T0"
        );
    }

    /// 기존 실제 deadlock은 여전히 탐지되는지 회귀 테스트
    #[test]
    fn test_real_deadlock_still_detected() {
        // T0: Req(R0) → Req(R1), T1: Req(R1) → Req(R0) = AB-BA
        let mut state = KiState::initial(2, 2);

        // T0 holds R0
        state.apply_operation(ThreadId(0), Operation::Request, ResourceId(0));
        // T1 holds R1
        state.apply_operation(ThreadId(1), Operation::Request, ResourceId(1));
        // T0 waits for R1 (held by T1)
        state.apply_operation(ThreadId(0), Operation::Request, ResourceId(1));
        assert_eq!(state.thread_status[0], ThreadStatus::Blocked);
        // T1 waits for R0 (held by T0)
        state.apply_operation(ThreadId(1), Operation::Request, ResourceId(0));
        assert_eq!(state.thread_status[1], ThreadStatus::Blocked);

        // Both Blocked AND holding each other's resource → REAL deadlock
        let cycle = state.detect_wfg_cycle();
        assert!(cycle.is_some(), "Real AB-BA deadlock must be detected");
    }
}
