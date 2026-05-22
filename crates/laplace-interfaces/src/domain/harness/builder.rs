// SPDX-License-Identifier: Apache-2.0
//! Builder API for concise deadlock probe specifications.

use super::spec::{
    GateId, GateSpec, HarnessSpec, ReleasePolicy, ResourceId, ResourceSpec, ThreadAction,
    ThreadSpec,
};

/// Builder entry point for verification probes.
#[derive(Debug, Clone, Default)]
pub struct DeadlockProbe {
    threads: Vec<ThreadSpec>,
    resources: Vec<ResourceSpec>,
    sync_points: Vec<GateSpec>,
}

impl DeadlockProbe {
    /// Create an empty probe builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin defining a thread.
    pub fn thread(self, id: &str) -> ThreadBuilder {
        ThreadBuilder {
            probe: self,
            thread: ThreadSpec {
                id: id.to_string(),
                actions: Vec::new(),
            },
        }
    }

    /// Begin defining a resource.
    pub fn resource(self, id: &str) -> ResourceBuilder {
        ResourceBuilder {
            probe: self,
            resource: ResourceSpec {
                id: ResourceId::new(id),
            },
        }
    }

    /// Begin defining a gate.
    pub fn gate(self, id: &str) -> GateBuilder {
        GateBuilder {
            probe: self,
            id: GateId::new(id),
            wait_for: 1,
            policy: ReleasePolicy::WaitFor,
        }
    }

    /// Build the immutable harness spec.
    pub fn build(self) -> HarnessSpec {
        HarnessSpec {
            threads: self.threads,
            resources: self.resources,
            sync_points: self.sync_points,
        }
    }
}

/// Thread-specific builder.
#[derive(Debug, Clone)]
pub struct ThreadBuilder {
    probe: DeadlockProbe,
    thread: ThreadSpec,
}

impl ThreadBuilder {
    /// Add a gate wait action.
    pub fn gate(mut self, id: &str) -> Self {
        self.thread.actions.push(ThreadAction::Gate {
            id: GateId::new(id),
        });
        self
    }

    /// Add a gate release action.
    pub fn release_gate(mut self, id: &str) -> Self {
        self.thread.actions.push(ThreadAction::ReleaseGate {
            id: GateId::new(id),
        });
        self
    }

    /// Add a resource acquisition.
    pub fn acquires(mut self, id: &str) -> Self {
        self.thread.actions.push(ThreadAction::Acquire {
            resource: ResourceId::new(id),
        });
        self
    }

    /// Add a subsequent resource acquisition.
    pub fn then_acquires(self, id: &str) -> Self {
        self.acquires(id)
    }

    /// Add a resource release.
    pub fn releases(mut self, id: &str) -> Self {
        self.thread.actions.push(ThreadAction::Release {
            resource: ResourceId::new(id),
        });
        self
    }

    /// Finish this thread and return to the probe builder.
    pub fn done(mut self) -> DeadlockProbe {
        self.thread.actions.push(ThreadAction::Finish);
        self.probe.threads.push(self.thread);
        self.probe
    }
}

/// Resource-specific builder.
#[derive(Debug, Clone)]
pub struct ResourceBuilder {
    probe: DeadlockProbe,
    resource: ResourceSpec,
}

impl ResourceBuilder {
    /// Finish this resource and return to the probe builder.
    pub fn done(mut self) -> DeadlockProbe {
        self.probe.resources.push(self.resource);
        self.probe
    }
}

/// Gate-specific builder.
#[derive(Debug, Clone)]
pub struct GateBuilder {
    probe: DeadlockProbe,
    id: GateId,
    wait_for: usize,
    policy: ReleasePolicy,
}

impl GateBuilder {
    /// Set the participant count.
    pub fn wait_for(mut self, wait_for: usize) -> Self {
        self.wait_for = wait_for;
        self
    }

    /// Mark this gate as a simultaneous release gate.
    pub fn simultaneous(mut self) -> Self {
        self.policy = ReleasePolicy::Simultaneous;
        self
    }

    /// Finish this gate and return to the probe builder.
    pub fn done(mut self) -> DeadlockProbe {
        self.probe.sync_points.push(GateSpec {
            id: self.id,
            wait_for: self.wait_for,
            policy: self.policy,
        });
        self.probe
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deadlock_probe_builder_detects_abba() {
        let spec = DeadlockProbe::new()
            .resource("m1")
            .done()
            .resource("m2")
            .done()
            .thread("t1")
            .acquires("m1")
            .then_acquires("m2")
            .done()
            .thread("t2")
            .acquires("m2")
            .then_acquires("m1")
            .done()
            .build();

        assert!(spec.detects_abba_deadlock());
    }

    #[test]
    fn test_deadlock_probe_builder_no_fp_on_ordered_acquisition() {
        let spec = DeadlockProbe::new()
            .thread("t1")
            .acquires("m1")
            .then_acquires("m2")
            .done()
            .thread("t2")
            .acquires("m1")
            .then_acquires("m2")
            .done()
            .build();

        assert!(!spec.detects_abba_deadlock());
    }
}
