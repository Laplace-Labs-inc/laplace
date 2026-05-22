// SPDX-License-Identifier: Apache-2.0
//! Serializable harness specification model.

use serde::{Deserialize, Serialize};

/// Stable gate identifier used by the harness DSL.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GateId(pub String);

impl GateId {
    /// Create a new gate id.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// Stable resource identifier used by probe builders and DSL directives.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ResourceId(pub String);

impl ResourceId {
    /// Create a new resource id.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// Gate release policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReleasePolicy {
    /// Wait until the configured participant count arrives.
    WaitFor,
    /// Release all waiters together.
    Simultaneous,
}

/// Synchronization point declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateSpec {
    /// Gate identifier.
    pub id: GateId,
    /// Number of participants required before the gate can open.
    pub wait_for: usize,
    /// Release behavior.
    pub policy: ReleasePolicy,
}

/// Resource declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceSpec {
    /// Resource identifier.
    pub id: ResourceId,
}

/// A single thread-level harness action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThreadAction {
    /// Wait at a named gate.
    Gate {
        /// Gate id to wait on.
        id: GateId,
    },
    /// Release a named gate.
    ReleaseGate {
        /// Gate id to release.
        id: GateId,
    },
    /// Acquire a resource.
    Acquire {
        /// Resource id to acquire.
        resource: ResourceId,
    },
    /// Release a resource.
    Release {
        /// Resource id to release.
        resource: ResourceId,
    },
    /// Mark the thread complete.
    Finish,
}

/// One logical thread in a harness scenario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadSpec {
    /// Thread identifier.
    pub id: String,
    /// Ordered actions executed by this thread.
    pub actions: Vec<ThreadAction>,
}

/// Complete verification harness specification.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessSpec {
    /// Thread programs.
    pub threads: Vec<ThreadSpec>,
    /// Declared resources.
    pub resources: Vec<ResourceSpec>,
    /// Synchronization points.
    pub sync_points: Vec<GateSpec>,
}

impl HarnessSpec {
    /// Returns true when two threads acquire the same pair of resources in
    /// opposite order before releasing either resource.
    pub fn detects_abba_deadlock(&self) -> bool {
        for i in 0..self.threads.len() {
            for j in (i + 1)..self.threads.len() {
                if acquisition_pairs_conflict(&self.threads[i], &self.threads[j]) {
                    return true;
                }
            }
        }
        false
    }
}

fn acquisition_pairs_conflict(a: &ThreadSpec, b: &ThreadSpec) -> bool {
    let a_pairs = held_acquisition_pairs(a);
    let b_pairs = held_acquisition_pairs(b);

    a_pairs.iter().any(|(first, second)| {
        b_pairs
            .iter()
            .any(|(other_first, other_second)| first == other_second && second == other_first)
    })
}

fn held_acquisition_pairs(thread: &ThreadSpec) -> Vec<(ResourceId, ResourceId)> {
    let mut held = Vec::<ResourceId>::new();
    let mut pairs = Vec::new();

    for action in &thread.actions {
        match action {
            ThreadAction::Acquire { resource } => {
                for prior in &held {
                    pairs.push((prior.clone(), resource.clone()));
                }
                if !held.contains(resource) {
                    held.push(resource.clone());
                }
            }
            ThreadAction::Release { resource } => {
                held.retain(|held_resource| held_resource != resource);
            }
            ThreadAction::Gate { .. } | ThreadAction::ReleaseGate { .. } | ThreadAction::Finish => {
            }
        }
    }

    pairs
}
