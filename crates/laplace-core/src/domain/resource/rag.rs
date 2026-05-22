// SPDX-License-Identifier: Apache-2.0
//! Resource Allocation Graph tracker with counting semaphore support.

use super::pearce_kelly::{PearceKellyState, PK_MAX_NODES};
use super::tracker::ResourceTracker;
use super::types::*;
use laplace_interfaces::domain::resource::types::ResourceCapacity;
use std::collections::VecDeque;

pub const RAG_MAX_THREADS: usize = 8;
pub const RAG_MAX_RESOURCES: usize = 8;

/// Internal RAG resource node state.
#[derive(Debug, Clone, Copy)]
struct ResourceNode {
    capacity: u32,
    held: u32,
}

impl ResourceNode {
    fn new(capacity: u32) -> Self {
        debug_assert!(capacity >= 1);
        Self { capacity, held: 0 }
    }

    fn is_available(&self) -> bool {
        self.held < self.capacity
    }
}

impl Default for ResourceNode {
    fn default() -> Self {
        Self::new(ResourceCapacity::MUTEX.as_u32())
    }
}

/// RAG-based resource tracker.
///
/// # TLA+ correspondence
///
/// ```tla
/// VARIABLES
///   resource_nodes: [Resource -> [capacity: Nat, held: Nat, holders: SUBSET Thread]],
///   waiting_queues: [Resource -> Seq(Thread)],
///   wait_for_graph: [Thread -> SUBSET Thread],
///   thread_status: [Thread -> {"Running","Blocked","Finished"}]
///
/// INVARIANTS
///   SemaphoreCapacity(r) == resource_nodes[r].held <= resource_nodes[r].capacity
///   MutexExclusive(r) == resource_nodes[r].capacity = 1 =>
///       Cardinality(resource_nodes[r].holders) <= 1
/// ```
#[derive(Debug)]
pub struct RagTracker {
    num_threads: usize,
    num_resources: usize,
    nodes: [ResourceNode; RAG_MAX_RESOURCES],
    /// Resource holders bitmask: thread i maps to bit i.
    holders: [u64; RAG_MAX_RESOURCES],
    waiting_queues: [VecDeque<ThreadId>; RAG_MAX_RESOURCES],
    /// wait_for_graph[t1][t2] = t1 waits for t2.
    wait_for_graph: [[bool; RAG_MAX_THREADS]; RAG_MAX_THREADS],
    thread_status: [ThreadStatus; RAG_MAX_THREADS],
    context_switches: u32,
}

impl RagTracker {
    /// Initializes all resources as mutexes (`capacity = 1`).
    pub fn new(num_threads: usize, num_resources: usize) -> Self {
        <Self as ResourceTracker>::new(num_threads, num_resources)
    }

    /// Initializes resources with explicit capacities.
    pub fn new_with_capacities(num_threads: usize, capacities: &[ResourceCapacity]) -> Self {
        assert!(
            num_threads <= RAG_MAX_THREADS,
            "Too many threads: {} (max {})",
            num_threads,
            RAG_MAX_THREADS
        );
        assert!(
            capacities.len() <= RAG_MAX_RESOURCES,
            "Too many resources: {} (max {})",
            capacities.len(),
            RAG_MAX_RESOURCES
        );

        let mut nodes = [ResourceNode::default(); RAG_MAX_RESOURCES];
        for (idx, capacity) in capacities.iter().enumerate() {
            nodes[idx] = ResourceNode::new(capacity.as_u32());
        }

        Self {
            num_threads,
            num_resources: capacities.len(),
            nodes,
            holders: [0; RAG_MAX_RESOURCES],
            waiting_queues: Default::default(),
            wait_for_graph: [[false; RAG_MAX_THREADS]; RAG_MAX_THREADS],
            thread_status: [ThreadStatus::Running; RAG_MAX_THREADS],
            context_switches: 0,
        }
    }

    /// Finds all current wait-for cycles.
    pub fn find_all_cycles(&self) -> Vec<Vec<ThreadId>> {
        find_all_cycles_in(&self.wait_for_graph, self.num_threads)
    }

    fn validate_thread(&self, thread: ThreadId) -> Result<usize, ResourceError> {
        let idx = thread.as_usize();
        if idx >= self.num_threads {
            Err(ResourceError::InvalidThreadId(thread))
        } else {
            Ok(idx)
        }
    }

    fn validate_resource(&self, resource: ResourceId) -> Result<usize, ResourceError> {
        let idx = resource.as_usize();
        if idx >= self.num_resources {
            Err(ResourceError::InvalidResourceId(resource))
        } else {
            Ok(idx)
        }
    }

    fn holder_threads(&self, resource_idx: usize) -> Vec<ThreadId> {
        (0..self.num_threads)
            .filter(|&idx| self.holders[resource_idx] & bit(idx) != 0)
            .map(ThreadId)
            .collect()
    }

    fn remove_wait_edges_to_holders(
        graph: &mut [[bool; RAG_MAX_THREADS]; RAG_MAX_THREADS],
        waiter: ThreadId,
        holders: &[ThreadId],
    ) {
        let waiter_idx = waiter.as_usize();
        for holder in holders {
            graph[waiter_idx][holder.as_usize()] = false;
        }
    }
}

impl ResourceTracker for RagTracker {
    fn new(num_threads: usize, num_resources: usize) -> Self {
        Self::new_with_capacities(num_threads, &vec![ResourceCapacity::MUTEX; num_resources])
    }

    fn request(
        &mut self,
        thread: ThreadId,
        resource: ResourceId,
    ) -> Result<RequestResult, ResourceError> {
        let thread_idx = self.validate_thread(thread)?;
        let resource_idx = self.validate_resource(resource)?;
        let thread_bit = bit(thread_idx);

        if self.holders[resource_idx] & thread_bit != 0 {
            return Err(ResourceError::AlreadyOwned { thread, resource });
        }

        if self.nodes[resource_idx].is_available() {
            self.holders[resource_idx] |= thread_bit;
            self.nodes[resource_idx].held += 1;
            return Ok(RequestResult::Acquired);
        }

        let holders = self.holder_threads(resource_idx);
        let mut candidate_graph = self.wait_for_graph;
        let mut pk = PearceKellyState::new(self.num_threads.min(PK_MAX_NODES));
        let mut would_create_cycle = false;
        for holder in &holders {
            would_create_cycle |= pk.insert_edge(
                &to_pk_graph(&candidate_graph),
                thread_idx,
                holder.as_usize(),
            );
            candidate_graph[thread_idx][holder.as_usize()] = true;
        }

        let cycles = find_all_cycles_in(&candidate_graph, self.num_threads);
        if would_create_cycle || !cycles.is_empty() {
            return if cycles.len() > 1 {
                Err(ResourceError::MultiCycleDeadlock { cycles })
            } else {
                Err(ResourceError::DeadlockDetected {
                    cycle: cycles.into_iter().next().unwrap(),
                })
            };
        }

        self.waiting_queues[resource_idx].push_back(thread);
        self.wait_for_graph = candidate_graph;
        self.thread_status[thread_idx] = ThreadStatus::Blocked;
        self.context_switches += 1;

        Ok(RequestResult::Blocked)
    }

    fn release(&mut self, thread: ThreadId, resource: ResourceId) -> Result<(), ResourceError> {
        let thread_idx = self.validate_thread(thread)?;
        let resource_idx = self.validate_resource(resource)?;
        let thread_bit = bit(thread_idx);

        if self.holders[resource_idx] & thread_bit == 0 {
            return Err(ResourceError::NotOwned { thread, resource });
        }

        let previous_holders = self.holder_threads(resource_idx);
        self.holders[resource_idx] &= !thread_bit;
        self.nodes[resource_idx].held -= 1;

        if self.nodes[resource_idx].is_available() {
            if let Some(next_thread) = self.waiting_queues[resource_idx].pop_front() {
                let next_idx = next_thread.as_usize();
                let next_bit = bit(next_idx);
                self.holders[resource_idx] |= next_bit;
                self.nodes[resource_idx].held += 1;
                self.thread_status[next_idx] = ThreadStatus::Running;
                Self::remove_wait_edges_to_holders(
                    &mut self.wait_for_graph,
                    next_thread,
                    &previous_holders,
                );

                let current_holders = self.holder_threads(resource_idx);
                for waiter in &self.waiting_queues[resource_idx] {
                    let waiter_idx = waiter.as_usize();
                    for old_holder in &previous_holders {
                        if self.holders[resource_idx] & bit(old_holder.as_usize()) == 0 {
                            self.wait_for_graph[waiter_idx][old_holder.as_usize()] = false;
                        }
                    }
                    for holder in &current_holders {
                        if *holder != *waiter {
                            self.wait_for_graph[waiter_idx][holder.as_usize()] = true;
                        }
                    }
                }
            }
        }

        self.context_switches += 1;
        Ok(())
    }

    fn on_finish(&mut self, thread: ThreadId) -> Result<(), ResourceError> {
        let thread_idx = self.validate_thread(thread)?;
        let held_resources: Vec<ResourceId> = (0..self.num_resources)
            .filter(|&resource_idx| self.holders[resource_idx] & bit(thread_idx) != 0)
            .map(ResourceId)
            .collect();

        if !held_resources.is_empty() {
            return Err(ResourceError::ResourceLeak {
                thread,
                held_resources,
            });
        }

        self.thread_status[thread_idx] = ThreadStatus::Finished;
        Ok(())
    }

    fn has_deadlock(&self) -> bool {
        !self.find_all_cycles().is_empty()
    }

    fn deadlocked_threads(&self) -> Vec<ThreadId> {
        let mut threads = Vec::new();
        for cycle in self.find_all_cycles() {
            for thread in cycle {
                if !threads.contains(&thread) {
                    threads.push(thread);
                }
            }
        }
        threads
    }

    fn contention_score(&self) -> u32 {
        self.waiting_queues
            .iter()
            .take(self.num_resources)
            .map(|queue| queue.len() as u32)
            .sum()
    }

    fn interleaving_score(&self) -> u32 {
        self.context_switches
    }
}

fn bit(thread_idx: usize) -> u64 {
    1u64 << thread_idx
}

fn to_pk_graph(
    graph: &[[bool; RAG_MAX_THREADS]; RAG_MAX_THREADS],
) -> [[bool; PK_MAX_NODES]; PK_MAX_NODES] {
    let mut pk_graph = [[false; PK_MAX_NODES]; PK_MAX_NODES];
    for row in 0..PK_MAX_NODES.min(RAG_MAX_THREADS) {
        for col in 0..PK_MAX_NODES.min(RAG_MAX_THREADS) {
            pk_graph[row][col] = graph[row][col];
        }
    }
    pk_graph
}

fn find_all_cycles_in(
    graph: &[[bool; RAG_MAX_THREADS]; RAG_MAX_THREADS],
    n: usize,
) -> Vec<Vec<ThreadId>> {
    let sccs = tarjan_scc(graph, n);
    let mut cycles = Vec::new();

    for scc in sccs {
        if scc.len() == 1 {
            let node = scc[0];
            if graph[node][node] {
                cycles.push(vec![ThreadId(node)]);
            }
            continue;
        }

        for &from in &scc {
            for &to in &scc {
                if graph[from][to] {
                    if let Some(mut path) = find_path_within_scc(graph, n, to, from, &scc) {
                        let mut cycle = vec![ThreadId(from)];
                        cycle.append(&mut path);
                        if !contains_cycle(&cycles, &cycle) {
                            cycles.push(cycle);
                        }
                    }
                }
            }
        }
    }

    cycles
}

fn tarjan_scc(graph: &[[bool; RAG_MAX_THREADS]; RAG_MAX_THREADS], n: usize) -> Vec<Vec<usize>> {
    struct Tarjan<'a> {
        graph: &'a [[bool; RAG_MAX_THREADS]; RAG_MAX_THREADS],
        n: usize,
        index: usize,
        indexes: [Option<usize>; RAG_MAX_THREADS],
        lowlinks: [usize; RAG_MAX_THREADS],
        stack: Vec<usize>,
        on_stack: [bool; RAG_MAX_THREADS],
        sccs: Vec<Vec<usize>>,
    }

    impl<'a> Tarjan<'a> {
        fn strong_connect(&mut self, node: usize) {
            self.indexes[node] = Some(self.index);
            self.lowlinks[node] = self.index;
            self.index += 1;
            self.stack.push(node);
            self.on_stack[node] = true;

            for next in 0..self.n {
                if !self.graph[node][next] {
                    continue;
                }
                if self.indexes[next].is_none() {
                    self.strong_connect(next);
                    self.lowlinks[node] = self.lowlinks[node].min(self.lowlinks[next]);
                } else if self.on_stack[next] {
                    self.lowlinks[node] = self.lowlinks[node].min(self.indexes[next].unwrap());
                }
            }

            if self.lowlinks[node] == self.indexes[node].unwrap() {
                let mut scc = Vec::new();
                while let Some(member) = self.stack.pop() {
                    self.on_stack[member] = false;
                    scc.push(member);
                    if member == node {
                        break;
                    }
                }
                self.sccs.push(scc);
            }
        }
    }

    let mut tarjan = Tarjan {
        graph,
        n,
        index: 0,
        indexes: [None; RAG_MAX_THREADS],
        lowlinks: [0; RAG_MAX_THREADS],
        stack: Vec::new(),
        on_stack: [false; RAG_MAX_THREADS],
        sccs: Vec::new(),
    };

    for node in 0..n {
        if tarjan.indexes[node].is_none() {
            tarjan.strong_connect(node);
        }
    }

    tarjan.sccs
}

fn find_path_within_scc(
    graph: &[[bool; RAG_MAX_THREADS]; RAG_MAX_THREADS],
    n: usize,
    start: usize,
    target: usize,
    scc: &[usize],
) -> Option<Vec<ThreadId>> {
    let mut visited = [false; RAG_MAX_THREADS];
    let mut path = Vec::new();
    if dfs_path(graph, n, start, target, scc, &mut visited, &mut path) {
        Some(path)
    } else {
        None
    }
}

fn dfs_path(
    graph: &[[bool; RAG_MAX_THREADS]; RAG_MAX_THREADS],
    n: usize,
    current: usize,
    target: usize,
    scc: &[usize],
    visited: &mut [bool; RAG_MAX_THREADS],
    path: &mut Vec<ThreadId>,
) -> bool {
    if visited[current] {
        return false;
    }
    visited[current] = true;
    path.push(ThreadId(current));

    if current == target {
        return true;
    }

    for next in 0..n {
        if graph[current][next]
            && scc.contains(&next)
            && dfs_path(graph, n, next, target, scc, visited, path)
        {
            return true;
        }
    }

    path.pop();
    false
}

fn contains_cycle(cycles: &[Vec<ThreadId>], candidate: &[ThreadId]) -> bool {
    let candidate_key = canonical_cycle_key(candidate);
    cycles
        .iter()
        .any(|cycle| canonical_cycle_key(cycle) == candidate_key)
}

fn canonical_cycle_key(cycle: &[ThreadId]) -> Vec<usize> {
    let mut ids: Vec<usize> = cycle.iter().map(|thread| thread.as_usize()).collect();
    if ids.len() > 1 && ids.first() == ids.last() {
        ids.pop();
    }
    if ids.is_empty() {
        return ids;
    }

    let start = ids
        .iter()
        .enumerate()
        .min_by_key(|(_, id)| *id)
        .map(|(idx, _)| idx)
        .unwrap();
    ids.rotate_left(start);
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semaphore_capacity_2_allows_two_holders() {
        let mut tracker = RagTracker::new_with_capacities(3, &[ResourceCapacity::new(2)]);

        assert_eq!(
            tracker.request(ThreadId(0), ResourceId(0)).unwrap(),
            RequestResult::Acquired
        );
        assert_eq!(
            tracker.request(ThreadId(1), ResourceId(0)).unwrap(),
            RequestResult::Acquired
        );
        assert_eq!(tracker.contention_score(), 0);
    }

    #[test]
    fn test_semaphore_capacity_2_blocks_third_requester() {
        let mut tracker = RagTracker::new_with_capacities(3, &[ResourceCapacity::new(2)]);
        tracker.request(ThreadId(0), ResourceId(0)).unwrap();
        tracker.request(ThreadId(1), ResourceId(0)).unwrap();

        assert_eq!(
            tracker.request(ThreadId(2), ResourceId(0)).unwrap(),
            RequestResult::Blocked
        );
        assert_eq!(tracker.contention_score(), 1);
    }

    #[test]
    fn test_semaphore_deadlock_when_all_slots_held() {
        let mut tracker = RagTracker::new_with_capacities(
            3,
            &[ResourceCapacity::new(2), ResourceCapacity::MUTEX],
        );
        tracker.request(ThreadId(0), ResourceId(0)).unwrap();
        tracker.request(ThreadId(1), ResourceId(0)).unwrap();
        tracker.request(ThreadId(2), ResourceId(1)).unwrap();
        assert_eq!(
            tracker.request(ThreadId(2), ResourceId(0)).unwrap(),
            RequestResult::Blocked
        );

        let result = tracker.request(ThreadId(0), ResourceId(1));
        assert!(matches!(
            result,
            Err(ResourceError::DeadlockDetected { .. })
        ));
    }

    #[test]
    fn test_multi_cycle_deadlock_3_threads() {
        let mut tracker = RagTracker::new_with_capacities(
            3,
            &[
                ResourceCapacity::new(2),
                ResourceCapacity::MUTEX,
                ResourceCapacity::MUTEX,
            ],
        );
        tracker.request(ThreadId(0), ResourceId(0)).unwrap();
        tracker.request(ThreadId(1), ResourceId(0)).unwrap();
        tracker.request(ThreadId(2), ResourceId(1)).unwrap();
        tracker.request(ThreadId(2), ResourceId(2)).unwrap();
        assert_eq!(
            tracker.request(ThreadId(0), ResourceId(1)).unwrap(),
            RequestResult::Blocked
        );
        assert_eq!(
            tracker.request(ThreadId(1), ResourceId(2)).unwrap(),
            RequestResult::Blocked
        );

        let result = tracker.request(ThreadId(2), ResourceId(0));
        match result {
            Err(ResourceError::MultiCycleDeadlock { cycles }) => {
                assert!(
                    cycles.len() >= 2,
                    "expected multiple cycles, got {cycles:?}"
                );
            }
            other => panic!("expected multi-cycle deadlock, got {other:?}"),
        }
    }

    #[test]
    fn test_rag_mutex_behaves_like_wfg() {
        let mut tracker = RagTracker::new(2, 2);
        assert_eq!(
            tracker.request(ThreadId(0), ResourceId(0)).unwrap(),
            RequestResult::Acquired
        );
        assert_eq!(
            tracker.request(ThreadId(1), ResourceId(1)).unwrap(),
            RequestResult::Acquired
        );
        assert_eq!(
            tracker.request(ThreadId(0), ResourceId(1)).unwrap(),
            RequestResult::Blocked
        );

        let result = tracker.request(ThreadId(1), ResourceId(0));
        assert!(matches!(
            result,
            Err(ResourceError::DeadlockDetected { .. })
        ));
    }
}
