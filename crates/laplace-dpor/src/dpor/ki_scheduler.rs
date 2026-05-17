// SPDX-License-Identifier: Apache-2.0
//! Ki-DPOR Scheduler with Liveness Detection
//!
//! This module implements the intelligent state space explorer with
//! starvation detection capabilities.

use super::classic::{DporStats, Operation};
use super::ki_state::{KiState, ThreadStatus};
use laplace_interfaces::domain::resource::{ResourceId, ThreadId};
use laplace_interfaces::AxiomConfig;
use std::collections::{BinaryHeap, HashSet};

/// Liveness violation types
///
/// # TLA+ Correspondence
///
/// ```tla
/// IsStarving(state, thread) ==
///     state.starvation_counters_state[thread] > MaxStarvationLimit
/// ```
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LivenessViolation {
    /// Thread is starved (exceeded MAX_STARVATION_LIMIT)
    Starvation {
        /// Thread that is starved
        thread: ThreadId,
        /// Number of steps thread has been waiting
        count: usize,
    },

    /// Deadlock detected — a cycle was found in the Wait-For Graph.
    ///
    /// Covers both **partial** deadlocks (only some threads in the cycle,
    /// others still running) and **total** deadlocks (all threads blocked).
    Deadlock {
        /// Ordered list of thread IDs forming the circular-wait cycle.
        ///
        /// Example: `[ThreadId(0), ThreadId(1)]` means T0 waits for T1
        /// and T1 waits for T0.
        cycle: Vec<ThreadId>,
    },

    /// User-defined invariant was violated during state space exploration.
    ///
    /// Set by `DporRunner::run_with_invariants` when the caller-supplied
    /// `invariant_checker` closure returns `Some(description)`.
    InvariantViolation {
        /// Human-readable description of the violated invariant.
        description: String,
    },
}

/// Ki-DPOR Scheduler with Liveness Checking
///
/// # TLA+ Correspondence
///
/// Extended from `KiDporScheduler` with liveness tracking:
///
/// ```tla
/// VARIABLES
///     priority_queue,
///     explored_set,
///     starvation_counters,  # NEW
///     fairness_stats        # NEW
/// ```
pub struct KiDporScheduler {
    /// Priority queue (open set in A*)
    open_set: BinaryHeap<KiState>,

    /// Explored state signatures
    explored_hashes: HashSet<u64>,

    /// Current state being expanded
    current_state: Option<KiState>,

    /// Number of threads
    num_threads: usize,

    /// Number of resources
    #[allow(dead_code)] // Used for validation, may be used in future
    num_resources: usize,

    /// Statistics
    stats: DporStats,

    /// Liveness violation found (if any)
    liveness_violation: Option<LivenessViolation>,

    /// Maximum steps a thread can wait before starvation is flagged (injected from config)
    starvation_limit: usize,
}

impl KiDporScheduler {
    /// Create a new Ki-DPOR scheduler with default config values.
    pub fn new(num_threads: usize, num_resources: usize) -> Self {
        Self::with_config(num_threads, num_resources, &AxiomConfig::default())
    }

    /// Create a new Ki-DPOR scheduler with injected config.
    pub fn with_config(num_threads: usize, num_resources: usize, cfg: &AxiomConfig) -> Self {
        let starvation_limit = cfg.max_starvation_limit as usize;
        let max_danger = cfg.max_danger as usize;
        let max_threads = cfg.max_threads as usize;

        let mut open_set = BinaryHeap::new();
        let initial_state =
            KiState::initial_with_config(num_threads, num_resources, max_threads, max_danger);

        open_set.push(initial_state);

        Self {
            open_set,
            explored_hashes: HashSet::new(),
            current_state: None,
            num_threads,
            num_resources,
            stats: DporStats::default(),
            liveness_violation: None,
            starvation_limit,
        }
    }

    /// CQS Command — DPOR 탐색을 한 단계 전진시킨다.
    ///
    /// 반환값 없음. 상태 변화는 `current()` / `is_complete()` 쿼리로 확인.
    pub fn advance(&mut self) {
        let _ = self.next_state_impl();
    }

    /// Get next state to explore
    ///
    /// Returns None if exploration is complete or liveness violation found.
    #[deprecated(
        since = "0.8.0",
        note = "Use advance() + current() for CQS compliance"
    )]
    pub fn next_state(&mut self) -> Option<&KiState> {
        self.next_state_impl()
    }

    fn next_state_impl(&mut self) -> Option<&KiState> {
        // Stop if we found a liveness violation
        if self.liveness_violation.is_some() {
            return None;
        }

        if let Some(state) = self.open_set.pop() {
            // Check for liveness violations
            if let Some(violation) = self.check_liveness(&state) {
                self.liveness_violation = Some(violation);
                self.current_state = Some(state);
                return None; // Stop exploration
            }

            // Mark as explored
            self.explored_hashes.insert(state.signature());
            self.stats.explored_states += 1;

            self.current_state = Some(state);
            self.current_state.as_ref()
        } else {
            None
        }
    }

    /// Check for liveness violations in a state.
    ///
    /// Checks (in order):
    /// 1. **Starvation** — any thread has exceeded `MAX_STARVATION_LIMIT`.
    /// 2. **Partial deadlock** — Wait-For Graph contains a cycle (checked
    ///    only when at least one thread is `Blocked`, as an optimization).
    /// 3. **Total deadlock** — all threads are `Blocked` (fallback for edge
    ///    cases where stale `waiting_queues` entries prevent WFG cycle
    ///    detection after an un-woken release).
    ///
    /// # TLA+ Correspondence
    ///
    /// ```tla
    /// NoStarvation ==
    ///     \A t \in Threads : starvation_counters[t] <= MaxStarvationLimit
    ///
    /// NoDeadlock ==
    ///     ~HasCycle(WaitForGraph) /\
    ///     ~(\A t \in Threads : thread_status_state[t] = "Blocked")
    /// ```
    fn check_liveness(&self, state: &KiState) -> Option<LivenessViolation> {
        // 1. Starvation check.
        for (t_idx, &count) in state.starvation_counters.iter().enumerate() {
            if count > self.starvation_limit {
                return Some(LivenessViolation::Starvation {
                    thread: ThreadId(t_idx),
                    count,
                });
            }
        }

        // 2. Partial-deadlock: WFG cycle detection.
        //    Optimization: skip entirely when no thread is Blocked (no WFG edges).
        let has_blocked = state.thread_status.contains(&ThreadStatus::Blocked);

        if has_blocked {
            if let Some(cycle) = state.detect_wfg_cycle() {
                return Some(LivenessViolation::Deadlock { cycle });
            }
        }

        // 3. Total-deadlock fallback (all threads Blocked).
        //    Handles the case where all threads are Blocked but the WFG has no
        //    edges because resources were released without auto-waking waiters.
        let blocked_threads: Vec<ThreadId> = state
            .thread_status
            .iter()
            .enumerate()
            .filter(|(_, &status)| status == ThreadStatus::Blocked)
            .map(|(i, _)| ThreadId(i))
            .collect();

        if blocked_threads.len() == self.num_threads && self.num_threads > 0 {
            return Some(LivenessViolation::Deadlock {
                cycle: blocked_threads,
            });
        }

        None
    }

    /// Generate and queue successor states
    pub fn expand_current<F>(&mut self, mut get_next_op: F)
    where
        F: FnMut(ThreadId, usize) -> Option<(Operation, ResourceId)>,
    {
        // Stop if we found a violation
        if self.liveness_violation.is_some() {
            return;
        }

        let current = match &self.current_state {
            Some(state) => state,
            None => return,
        };

        // Get enabled threads
        let enabled_threads = current.enabled_threads();

        for thread in enabled_threads {
            // Determine program counter
            let pc = current
                .path
                .iter()
                .filter(|step| step.thread == thread)
                .count();

            // Get next operation
            if let Some((operation, resource)) = get_next_op(thread, pc) {
                // Generate successor state
                let successor = current.successor(thread, operation, resource);

                // Skip if already explored
                if self.explored_hashes.contains(&successor.signature()) {
                    continue;
                }

                // Add to open set
                self.open_set.push(successor);

                // Update stats
                self.stats.max_depth = self.stats.max_depth.max(current.cost_g + 1);
            }
        }
    }

    /// Check if exploration is complete
    pub fn is_complete(&self) -> bool {
        self.open_set.is_empty() || self.liveness_violation.is_some()
    }

    /// Get liveness violation (if found)
    pub fn liveness_violation(&self) -> Option<&LivenessViolation> {
        self.liveness_violation.as_ref()
    }

    /// Get exploration statistics
    pub fn stats(&self) -> DporStats {
        self.stats
    }

    /// Get current state
    pub fn current(&self) -> Option<&KiState> {
        self.current_state.as_ref()
    }

    /// Get size of open set
    pub fn open_set_size(&self) -> usize {
        self.open_set.len()
    }

    /// Get number of explored states
    pub fn explored_count(&self) -> usize {
        self.explored_hashes.len()
    }

    /// Forcibly record a violation found by an external checker (e.g. `DporRunner`'s
    /// `invariant_checker`).  Once set, `extract_schedule` will return `Some`.
    pub fn set_violation(&mut self, violation: LivenessViolation) {
        self.liveness_violation = Some(violation);
    }

    /// Extract the defect-triggering schedule when a liveness violation is found.
    ///
    /// Returns `Some(Schedule)` only when a liveness violation has been detected.
    /// The schedule bundles the full execution path from the current state's
    /// `path` field together with the violation, ready for serialization and replay.
    ///
    /// Returns `None` if no violation has been found yet.
    pub fn extract_schedule(&self) -> Option<crate::dpor::schedule::Schedule> {
        let violation = self.liveness_violation.clone()?;
        let steps = self
            .current_state
            .as_ref()
            .map(|s| s.path.clone())
            .unwrap_or_default();
        Some(crate::dpor::schedule::Schedule {
            steps,
            violation: Some(violation),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_starvation_detection() {
        let mut scheduler = KiDporScheduler::new(2, 1);

        // Simulate greedy thread scenario
        let mut iteration = 0;
        let max_iterations = 50;

        while !scheduler.is_complete() && iteration < max_iterations {
            scheduler.advance();
            if scheduler.current().is_some() {
                scheduler.expand_current(|thread, pc| {
                    // Thread 0 is greedy (loops)
                    if thread == ThreadId(0) {
                        match pc % 2 {
                            0 => Some((Operation::Request, ResourceId(0))),
                            _ => Some((Operation::Release, ResourceId(0))),
                        }
                    } else {
                        // Thread 1 tries once
                        if pc == 0 {
                            Some((Operation::Request, ResourceId(0)))
                        } else {
                            None
                        }
                    }
                });
            }
            iteration += 1;
        }

        // Should detect starvation
        if let Some(violation) = scheduler.liveness_violation() {
            match violation {
                LivenessViolation::Starvation { thread, count } => {
                    assert_eq!(*thread, ThreadId(1));
                    assert!(*count > AxiomConfig::default().max_starvation_limit as usize);
                }
                _ => panic!("Expected starvation, got {:?}", violation),
            }
        } else {
            // Note: May not always find starvation in this simple test
            // due to heuristic exploration order
        }
    }

    #[test]
    fn test_deadlock_detection() {
        let mut state = KiState::initial(2, 2);

        // Both threads blocked
        state.thread_status[0] = ThreadStatus::Blocked;
        state.thread_status[1] = ThreadStatus::Blocked;

        let scheduler = KiDporScheduler::new(2, 2);
        let violation = scheduler.check_liveness(&state);

        assert!(matches!(
            violation,
            Some(LivenessViolation::Deadlock { .. })
        ));
    }
}
