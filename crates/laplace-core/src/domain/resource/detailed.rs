// SPDX-License-Identifier: Apache-2.0
//! DetailedTracker - Full Tracking for Verification
//!
//! This module provides a comprehensive resource tracking implementation designed
//! for use in the Axiom verification system. It maintains complete state about
//! resource ownership, waiting queues, and the wait-for graph to enable
//! detection of deadlocks and other concurrency bugs.

use super::tracker::*;
use super::types::*;
use std::collections::VecDeque;

/// Maximum threads for detailed tracking
///
/// Must match corresponding constant in DPOR module.
/// This limit exists for Kani verification compatibility.
pub const MAX_THREADS: usize = 8;

/// Maximum resources for detailed tracking
///
/// Verification workloads typically track a small number of resources.
/// This limit enables fixed-size arrays for Kani compatibility.
pub const MAX_RESOURCES: usize = 8;

/// Detailed resource tracker for verification
///
/// # TLA+ Correspondence
///
/// This implementation corresponds to the ResourceOracle specification:
///
/// ```tla
/// VARIABLES
///     resources: [Resource -> [owner: Thread | Null, count: Nat]],
///     waiting_queues: [Resource -> Seq(Thread)],
///     wait_for_graph: [Thread -> SUBSET Threads],
///     thread_status: [Thread -> {"Running", "Blocked", "Finished"}]
///
/// INVARIANTS
///     ThreadOwnsAtMostOne(t) ==
///         \E r \in Resources : resources[r].owner = t =>
///             \A r' \in Resources : r' # r => resources[r'].owner # t
///     NoWaitCycleWithoutOwner(g) ==
///         \A t \in Threads :
///             (wait_for_graph[t] # {}) => (\E r : resources[r].owner = t)
///     ValidWaitForGraph(g) ==
///         \A t1, t2 : wait_for_graph[t1][t2] =>
///             \E r : resources[r].owner = t2 /\ t1 \in waiting_queues[r]
/// ```
///
/// # Data Structures
///
/// The implementation uses fixed-size arrays for Kani verification compatibility:
///
/// - **Adjacency Matrix** for wait-for graph: `[bool; MAX_THREADS][MAX_THREADS]`
///   - O(1) space per edge, enabling O(1) cycle check setup
///   - Suitable for small thread counts (MAX_THREADS = 8)
/// - **Fixed-size arrays** for resource ownership and thread status
/// - **VecDeque** for FIFO waiting queues at each resource
#[derive(Debug)]
pub struct DetailedTracker {
    /// Number of threads currently tracked
    num_threads: usize,

    /// Number of resources currently tracked
    num_resources: usize,

    /// Resource ownership: resources[r] = Some(thread) if thread owns resource r
    /// TLA+: resources[r].owner
    resources: [Option<ThreadId>; MAX_RESOURCES],

    /// Waiting queues: waiting_queues[r] = [t1, t2, ...] threads waiting for resource r
    /// TLA+: waiting_queues[r]
    waiting_queues: [VecDeque<ThreadId>; MAX_RESOURCES],

    /// Wait-for graph as adjacency matrix
    /// wait_for_graph[t1][t2] = true means thread t1 waits for thread t2
    /// TLA+: wait_for_graph[t] = {threads that t waits for}
    wait_for_graph: [[bool; MAX_THREADS]; MAX_THREADS],

    /// Thread status: Running, Blocked, or Finished
    /// TLA+: thread_status[t]
    thread_status: [ThreadStatus; MAX_THREADS],

    /// Context switch counter for interleaving metrics
    /// Incremented each time a thread's status changes or waiting queues are modified
    context_switches: u32,
}

impl DetailedTracker {
    /// Check if adding an edge (from -> to) would create a cycle
    ///
    /// # TLA+ Correspondence
    ///
    /// This implements cycle detection from the HasCycle predicate:
    /// ```tla
    /// WouldCreateCycle(from, to) ==
    ///     to \in TransitiveClosure(wait_for_graph \cup {from -> to})[from]
    /// ```
    ///
    /// # Algorithm
    ///
    /// Uses depth-first search to find a path from `to` to `from`.
    /// If such a path exists, adding edge `from -> to` would complete a cycle.
    ///
    /// # Complexity
    ///
    /// O(V + E) where V = number of threads, E = current wait-for edges
    fn would_create_cycle(&self, from: ThreadId, to: ThreadId) -> Option<Vec<ThreadId>> {
        // Self-loop is always a cycle
        if from == to {
            return Some(vec![from]);
        }

        // DFS from 'to' to see if we can reach 'from'
        // If we can, then adding from -> to would create a cycle
        let mut visited = [false; MAX_THREADS];
        let mut path = Vec::new();

        if self.dfs_find_path(to, from, &mut visited) {
            // Found path from 'to' to 'from'
            // Adding edge 'from -> to' completes the cycle
            path.push(from);
            Some(path)
        } else {
            None
        }
    }

    /// DFS helper to find a path from current to target
    ///
    /// # Returns
    ///
    /// True if a path was found (path vector contains the path)
    /// False otherwise
    fn dfs_find_path(
        &self,
        current: ThreadId,
        target: ThreadId,
        visited: &mut [bool; MAX_THREADS],
    ) -> bool {
        let current_idx = current.as_usize();

        if current == target {
            return true;
        }

        if visited[current_idx] {
            return false;
        }

        visited[current_idx] = true;

        // Follow wait-for edges from current
        for next_idx in 0..self.num_threads {
            if self.wait_for_graph[current_idx][next_idx]
                && self.dfs_find_path(ThreadId(next_idx), target, visited)
            {
                return true;
            }
        }

        false
    }

    /// Find all cycles in the wait-for graph
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// AllCycles ==
    ///     {path : path[0] \in Threads /\
    ///             path \in Paths(wait_for_graph) /\
    ///             path[0] = path[Len(path)]}
    /// ```
    ///
    /// # Returns
    ///
    /// Vector of cycles, where each cycle is a path of threads.
    /// Empty if no cycles exist.
    fn find_cycles(&self) -> Vec<Vec<ThreadId>> {
        let mut cycles = Vec::new();

        for start_idx in 0..self.num_threads {
            let start_id = ThreadId(start_idx);

            // Skip if this thread has no outgoing wait-for edges
            let has_outgoing = (0..self.num_threads).any(|i| self.wait_for_graph[start_idx][i]);

            if !has_outgoing {
                continue;
            }

            let mut visited = [false; MAX_THREADS];
            let mut path = Vec::new();

            // DFS starting from this thread, looking for a path back to itself
            if self.dfs_cycle(start_id, start_id, &mut visited, &mut path, true) {
                cycles.push(path);
            }
        }

        cycles
    }

    /// DFS to detect cycles starting and ending at target
    ///
    /// # Arguments
    ///
    /// * `current` - Currently exploring from this thread
    /// * `target` - Looking for a cycle back to this thread
    /// * `visited` - Tracks visited threads in current DFS
    /// * `path` - Accumulates the path if a cycle is found
    /// * `is_start` - Whether this is the initial call (avoid immediate match)
    fn dfs_cycle(
        &self,
        current: ThreadId,
        target: ThreadId,
        visited: &mut [bool; MAX_THREADS],
        path: &mut Vec<ThreadId>,
        is_start: bool,
    ) -> bool {
        let current_idx = current.as_usize();

        // Found a cycle if we're back to target and not at start
        if !is_start && current == target {
            return true;
        }

        // Already visited in this DFS path
        if visited[current_idx] {
            return false;
        }

        visited[current_idx] = true;
        path.push(current);

        // Explore edges from current
        for next_idx in 0..self.num_threads {
            if self.wait_for_graph[current_idx][next_idx]
                && self.dfs_cycle(ThreadId(next_idx), target, visited, path, false)
            {
                return true;
            }
        }

        path.pop();
        false
    }
}

impl ResourceTracker for DetailedTracker {
    fn new(num_threads: usize, num_resources: usize) -> Self {
        assert!(
            num_threads <= MAX_THREADS,
            "Too many threads: {} (max {})",
            num_threads,
            MAX_THREADS
        );
        assert!(
            num_resources <= MAX_RESOURCES,
            "Too many resources: {} (max {})",
            num_resources,
            MAX_RESOURCES
        );

        // Initialize arrays with default values
        // We use inline initialization with array construction
        let resources = [None; MAX_RESOURCES];
        let waiting_queues: [VecDeque<ThreadId>; MAX_RESOURCES] = Default::default();
        let wait_for_graph = [[false; MAX_THREADS]; MAX_THREADS];
        let thread_status = [ThreadStatus::Running; MAX_THREADS];

        Self {
            num_threads,
            num_resources,
            resources,
            waiting_queues,
            wait_for_graph,
            thread_status,
            context_switches: 0,
        }
    }

    fn request(
        &mut self,
        thread: ThreadId,
        resource: ResourceId,
    ) -> Result<RequestResult, ResourceError> {
        // Validate bounds
        let thread_idx = thread.as_usize();
        let resource_idx = resource.as_usize();

        if thread_idx >= self.num_threads {
            return Err(ResourceError::InvalidThreadId(thread));
        }

        if resource_idx >= self.num_resources {
            return Err(ResourceError::InvalidResourceId(resource));
        }

        // TLA+ Invariant: Prevent self-deadlock
        // A thread cannot request a resource it already owns
        if self.resources[resource_idx] == Some(thread) {
            return Err(ResourceError::AlreadyOwned { thread, resource });
        }

        // Check if resource is available
        if let Some(owner) = self.resources[resource_idx] {
            // TLA+ Resource is held
            // Check if adding this wait-for edge would create a cycle
            if let Some(cycle) = self.would_create_cycle(thread, owner) {
                return Err(ResourceError::DeadlockDetected { cycle });
            }

            // Add thread to waiting queue (TLA+ Append)
            self.waiting_queues[resource_idx].push_back(thread);

            // Update wait-for graph (TLA+ wait_for_graph[thread] \cup {owner})
            self.wait_for_graph[thread_idx][owner.as_usize()] = true;

            // Update thread status (TLA+ thread_status[thread] := "Blocked")
            self.thread_status[thread_idx] = ThreadStatus::Blocked;

            // Track context switch
            self.context_switches += 1;

            Ok(RequestResult::Blocked)
        } else {
            // TLA+ Resource is free
            // Acquire immediately
            self.resources[resource_idx] = Some(thread);

            Ok(RequestResult::Acquired)
        }
    }

    fn release(&mut self, thread: ThreadId, resource: ResourceId) -> Result<(), ResourceError> {
        // Validate bounds
        let thread_idx = thread.as_usize();
        let resource_idx = resource.as_usize();

        if thread_idx >= self.num_threads {
            return Err(ResourceError::InvalidThreadId(thread));
        }

        if resource_idx >= self.num_resources {
            return Err(ResourceError::InvalidResourceId(resource));
        }

        // Check ownership (TLA+ resources[r].owner = thread)
        if self.resources[resource_idx] != Some(thread) {
            return Err(ResourceError::NotOwned { thread, resource });
        }

        // TLA+ ReleaseResource action
        if let Some(next_thread) = self.waiting_queues[resource_idx].pop_front() {
            // Wake up next waiter (TLA+ resources[r].owner := next_thread)
            self.resources[resource_idx] = Some(next_thread);

            // Update thread status (TLA+ thread_status[next_thread] := "Running")
            self.thread_status[next_thread.as_usize()] = ThreadStatus::Running;

            // Update wait-for graph (TLA+ wait_for_graph[next_thread] := @ \ {thread})
            self.wait_for_graph[next_thread.as_usize()][thread_idx] = false;

            self.context_switches += 1;
        } else {
            // No waiters, just release (TLA+ resources[r].owner := Null)
            self.resources[resource_idx] = None;
        }

        Ok(())
    }

    fn on_finish(&mut self, thread: ThreadId) -> Result<(), ResourceError> {
        let thread_idx = thread.as_usize();

        if thread_idx >= self.num_threads {
            return Err(ResourceError::InvalidThreadId(thread));
        }

        // TLA+ Invariant: Check that thread holds no resources
        let held: Vec<ResourceId> = (0..self.num_resources)
            .filter(|&r| self.resources[r] == Some(thread))
            .map(ResourceId)
            .collect();

        if !held.is_empty() {
            return Err(ResourceError::ResourceLeak {
                thread,
                held_resources: held,
            });
        }

        // TLA+ thread_status[thread] := "Finished"
        self.thread_status[thread_idx] = ThreadStatus::Finished;

        Ok(())
    }

    fn has_deadlock(&self) -> bool {
        // TLA+ HasCycle
        !self.find_cycles().is_empty()
    }

    fn deadlocked_threads(&self) -> Vec<ThreadId> {
        // TLA+ DeadlockedThreads = {t : t \in TransitiveClosure(wait_for_graph)[t]}
        let cycles = self.find_cycles();
        let mut threads = std::collections::HashSet::new();

        for cycle in cycles {
            for t in cycle {
                threads.insert(t);
            }
        }

        threads.into_iter().collect()
    }

    fn contention_score(&self) -> u32 {
        // TLA+ ContentionScore = Sum of |waiting_queues[r]| for all r
        // This measures how many threads are waiting
        self.waiting_queues
            .iter()
            .take(self.num_resources)
            .map(|q| q.len() as u32)
            .sum()
    }

    fn interleaving_score(&self) -> u32 {
        // Context switches count (higher = more complex interleaving)
        self.context_switches
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_acquire_release() {
        let mut tracker = DetailedTracker::new(2, 2);

        // Thread 0 acquires resource 0
        assert_eq!(
            tracker.request(ThreadId(0), ResourceId(0)).unwrap(),
            RequestResult::Acquired
        );

        // Thread 0 releases resource 0
        tracker.release(ThreadId(0), ResourceId(0)).unwrap();

        assert!(!tracker.has_deadlock());
    }

    #[test]
    fn test_self_deadlock_prevention() {
        let mut tracker = DetailedTracker::new(2, 2);

        // Thread 0 acquires resource 0
        tracker.request(ThreadId(0), ResourceId(0)).unwrap();

        // Thread 0 tries to acquire resource 0 again -> Error
        let result = tracker.request(ThreadId(0), ResourceId(0));
        assert!(matches!(result, Err(ResourceError::AlreadyOwned { .. })));
    }

    #[test]
    fn test_ab_ba_deadlock_detection() {
        let mut tracker = DetailedTracker::new(2, 2);

        // Thread 0 acquires r0
        tracker.request(ThreadId(0), ResourceId(0)).unwrap();

        // Thread 1 acquires r1
        tracker.request(ThreadId(1), ResourceId(1)).unwrap();

        // Thread 0 tries r1 -> Blocked
        assert_eq!(
            tracker.request(ThreadId(0), ResourceId(1)).unwrap(),
            RequestResult::Blocked
        );

        // Thread 1 tries r0 -> DEADLOCK!
        let result = tracker.request(ThreadId(1), ResourceId(0));
        assert!(matches!(
            result,
            Err(ResourceError::DeadlockDetected { .. })
        ));
    }

    #[test]
    fn test_resource_leak_detection() {
        let mut tracker = DetailedTracker::new(2, 2);

        // Thread 0 acquires resource 0
        tracker.request(ThreadId(0), ResourceId(0)).unwrap();

        // Thread 0 tries to finish without releasing -> Error
        let result = tracker.on_finish(ThreadId(0));
        assert!(matches!(result, Err(ResourceError::ResourceLeak { .. })));
    }

    #[test]
    fn test_contention_score() {
        let mut tracker = DetailedTracker::new(3, 1);

        // Thread 0 acquires the only resource
        tracker.request(ThreadId(0), ResourceId(0)).unwrap();
        assert_eq!(tracker.contention_score(), 0);

        // Thread 1 blocks waiting
        tracker.request(ThreadId(1), ResourceId(0)).unwrap();
        assert_eq!(tracker.contention_score(), 1);

        // Thread 2 also blocks
        tracker.request(ThreadId(2), ResourceId(0)).unwrap();
        assert_eq!(tracker.contention_score(), 2);

        // Thread 0 releases, next waiter runs
        tracker.release(ThreadId(0), ResourceId(0)).unwrap();
        assert_eq!(tracker.contention_score(), 1);
    }

    #[test]
    fn test_interleaving_score_increments() {
        let mut tracker = DetailedTracker::new(2, 1);

        let initial_score = tracker.interleaving_score(); // 0

        // 1. 단순 획득 (Acquired): 문맥 전환이 아니므로 스코어는 변하지 않음
        tracker.request(ThreadId(0), ResourceId(0)).unwrap();
        let after_acquire = tracker.interleaving_score();
        assert_eq!(after_acquire, initial_score);

        // 2. 차단 (Blocked): 스레드가 대기 상태로 들어가므로 스코어 증가
        tracker.request(ThreadId(1), ResourceId(0)).unwrap();
        let after_block = tracker.interleaving_score();
        assert!(after_block > after_acquire);

        // 3. 해제 및 깨움 (Release & Wake): 대기하던 스레드가 Running이 되므로 스코어 증가
        tracker.release(ThreadId(0), ResourceId(0)).unwrap();
        let after_release = tracker.interleaving_score();
        assert!(after_release > after_block);
    }
}
