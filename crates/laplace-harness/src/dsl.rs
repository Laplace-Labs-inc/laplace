// SPDX-License-Identifier: Apache-2.0
//! YAML/code DSL for synchronization directives.

use laplace_interfaces::{
    GateId, GateSpec, HarnessSpec, ReleasePolicy, ResourceSpec, ThreadAction, ThreadSpec,
};
use serde::{Deserialize, Serialize};

/// DSL directive consumed by harness builders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HarnessDirective {
    /// Synchronization gate declaration.
    Gate { id: GateId, wait_for: usize },
    /// Release all participants waiting at a gate.
    ReleaseGate { id: GateId },
    /// Acquire a resource.
    Acquire {
        resource: laplace_interfaces::HarnessResourceId,
    },
    /// Release a resource.
    Release {
        resource: laplace_interfaces::HarnessResourceId,
    },
    /// End of thread program.
    Finish,
}

/// YAML-friendly root document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessDsl {
    /// Thread programs.
    pub threads: Vec<ThreadDsl>,
    /// Optional resource declarations.
    #[serde(default)]
    pub resources: Vec<ResourceSpec>,
}

/// YAML-friendly thread program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadDsl {
    /// Thread id.
    pub id: String,
    /// Ordered directives.
    pub directives: Vec<HarnessDirective>,
}

impl HarnessDsl {
    /// Convert a DSL document into a reusable harness spec.
    pub fn into_spec(self) -> HarnessSpec {
        let mut sync_points = Vec::new();
        let threads = self
            .threads
            .into_iter()
            .map(|thread| {
                let mut actions = Vec::new();
                for directive in thread.directives {
                    match directive {
                        HarnessDirective::Gate { id, wait_for } => {
                            sync_points.push(GateSpec {
                                id: id.clone(),
                                wait_for,
                                policy: ReleasePolicy::WaitFor,
                            });
                            actions.push(ThreadAction::Gate { id });
                        }
                        HarnessDirective::ReleaseGate { id } => {
                            if let Some(gate) = sync_points.iter_mut().find(|gate| gate.id == id) {
                                gate.policy = ReleasePolicy::Simultaneous;
                            } else {
                                sync_points.push(GateSpec {
                                    id: id.clone(),
                                    wait_for: 1,
                                    policy: ReleasePolicy::Simultaneous,
                                });
                            }
                            actions.push(ThreadAction::ReleaseGate { id });
                        }
                        HarnessDirective::Acquire { resource } => {
                            actions.push(ThreadAction::Acquire { resource });
                        }
                        HarnessDirective::Release { resource } => {
                            actions.push(ThreadAction::Release { resource });
                        }
                        HarnessDirective::Finish => actions.push(ThreadAction::Finish),
                    }
                }
                ThreadSpec {
                    id: thread.id,
                    actions,
                }
            })
            .collect();

        HarnessSpec {
            threads,
            resources: self.resources,
            sync_points,
        }
    }

    /// Parse YAML into a harness spec.
    pub fn spec_from_yaml(input: &str) -> Result<HarnessSpec, serde_yaml::Error> {
        serde_yaml::from_str::<Self>(input).map(Self::into_spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use laplace_interfaces::ThreadAction;

    #[test]
    fn test_gate_synchronizes_n_threads() {
        let spec = HarnessDsl {
            resources: Vec::new(),
            threads: vec![ThreadDsl {
                id: "t1".to_string(),
                directives: vec![HarnessDirective::Gate {
                    id: GateId::new("start"),
                    wait_for: 2,
                }],
            }],
        }
        .into_spec();

        assert_eq!(spec.sync_points[0].wait_for, 2);
        assert_eq!(spec.sync_points[0].policy, ReleasePolicy::WaitFor);
    }

    #[test]
    fn test_release_gate_triggers_simultaneous_entry() {
        let spec = HarnessDsl {
            resources: Vec::new(),
            threads: vec![ThreadDsl {
                id: "t1".to_string(),
                directives: vec![
                    HarnessDirective::Gate {
                        id: GateId::new("start"),
                        wait_for: 2,
                    },
                    HarnessDirective::ReleaseGate {
                        id: GateId::new("start"),
                    },
                ],
            }],
        }
        .into_spec();

        assert_eq!(spec.sync_points[0].policy, ReleasePolicy::Simultaneous);
    }

    #[test]
    fn test_dsl_roundtrip_yaml_to_harnessspec() {
        let yaml = r#"
threads:
  - id: t1
    directives:
      - type: gate
        id: boot
        wait_for: 2
      - type: acquire
        resource: m1
      - type: release
        resource: m1
      - type: finish
resources:
  - id: m1
"#;
        let spec = HarnessDsl::spec_from_yaml(yaml).unwrap();

        assert_eq!(spec.threads.len(), 1);
        assert_eq!(spec.resources.len(), 1);
        assert!(matches!(
            spec.threads[0].actions[1],
            ThreadAction::Acquire { .. }
        ));
    }
}
