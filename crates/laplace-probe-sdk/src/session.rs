// SPDX-License-Identifier: Apache-2.0
//! Public in-memory probe event collection.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Mutex as StdMutex, OnceLock};

use crate::event::ProbeEvent;
use crate::license::load_axiom_max_depth;

thread_local! {
    /// Current OS-thread event sink. Threads without a registered sink drop events.
    static PROBE_SENDER: std::cell::RefCell<Option<mpsc::SyncSender<ProbeEvent>>> =
        const { std::cell::RefCell::new(None) };

    /// Current OS-thread logical thread id used by generated test harnesses.
    static PROBE_THREAD_ID: Cell<Option<u64>> = const { Cell::new(None) };
}

static GLOBAL_PROBE_SENDER: OnceLock<StdMutex<Option<mpsc::SyncSender<ProbeEvent>>>> =
    OnceLock::new();
static NEXT_IMPLICIT_THREAD_ID: AtomicU64 = AtomicU64::new(0);

/// Registers the current OS thread's probe event sink.
pub fn set_probe_sender(tx: mpsc::SyncSender<ProbeEvent>) {
    PROBE_SENDER.with(|s| *s.borrow_mut() = Some(tx.clone()));
    NEXT_IMPLICIT_THREAD_ID.store(0, Ordering::SeqCst);
    let slot = GLOBAL_PROBE_SENDER.get_or_init(|| StdMutex::new(None));
    *slot.lock().expect("global probe sender lock poisoned") = Some(tx);
}

/// Clears the current OS thread's probe event sink.
pub fn clear_probe_sender() {
    PROBE_SENDER.with(|s| *s.borrow_mut() = None);
    if let Some(slot) = GLOBAL_PROBE_SENDER.get() {
        *slot.lock().expect("global probe sender lock poisoned") = None;
    }
}

/// Assigns the current OS thread's logical probe thread id.
pub fn set_probe_thread_id(id: u64) {
    PROBE_THREAD_ID.with(|c| c.set(Some(id)));
}

/// Reads the current OS thread's logical probe thread id.
pub fn current_thread_id() -> u64 {
    PROBE_THREAD_ID.with(|slot| {
        if let Some(id) = slot.get() {
            return id;
        }

        // Annotated std-spawn model functions run child OS threads that did
        // not call the generated harness setup. Give those threads stable
        // logical ids so the in-memory reference verifier can still see the
        // lock-order relation. Explicit `set_probe_thread_id` remains the
        // preferred path for generated harnesses.
        let id = NEXT_IMPLICIT_THREAD_ID.fetch_add(1, Ordering::SeqCst);
        slot.set(Some(id));
        id
    })
}

/// Emits a public probe event to the registered thread-local sink.
pub fn emit(event: ProbeEvent) {
    let emitted_to_thread_local = PROBE_SENDER.with(|s| {
        if let Some(tx) = s.borrow().as_ref() {
            let _ = tx.send(event.clone());
            return true;
        }
        false
    });

    if !emitted_to_thread_local {
        if let Some(slot) = GLOBAL_PROBE_SENDER.get() {
            if let Some(tx) = slot
                .lock()
                .expect("global probe sender lock poisoned")
                .as_ref()
            {
                let _ = tx.send(event.clone());
            }
        }
    }

    #[cfg(feature = "cloud")]
    if let Some(client) = cloud::GLOBAL_PROBE_CLIENT.get() {
        if let Some(raw) = cloud::probe_event_to_raw(&event) {
            client.emit(raw);
        }
    }
}

/// Dumps captured events (with the declared expectation and determinism class)
/// to `$LAPLACE_VERIFY_EVENTS_DIR/<target>.json` when that env var is set — the
/// ingestion contract for the private CLI/API verdict boundary
/// (`laplace axiom verify --model-events <dir>`). No-op when the var is unset,
/// so it is invisible to a plain `cargo test`.
///
/// `determinism` is the producer's honest scope attestation for the captured
/// code (`fully_deterministic` | `deterministic_with_declared_inputs` |
/// `best_effort`), declared via `#[laplace::verify(determinism = "…")]`. The
/// captured trace replays bit-exactly, but whether that single trace represents
/// real input-dependent code is something only the author can attest; the
/// downstream `--strict` gate refuses to bless anything weaker than
/// `fully_deterministic`.
///
/// Trace data only: the verdict is computed downstream by the private engine,
/// never here. Best-effort — any I/O error is swallowed so verification runs are
/// never broken by a dump failure.
pub fn dump_events_if_configured(
    target_name: &str,
    expected: &str,
    determinism: &str,
    events: &[ProbeEvent],
) {
    let Ok(dir) = std::env::var("LAPLACE_VERIFY_EVENTS_DIR") else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = std::path::Path::new(&dir).join(format!("{target_name}.json"));
    let envelope = serde_json::json!({
        "target": target_name,
        "expected": expected,
        "determinism": determinism,
        "events": events,
    });
    if let Ok(text) = serde_json::to_string_pretty(&envelope) {
        let _ = std::fs::write(path, text);
    }
}

/// Probe collection configuration shared by generated public harnesses.
#[derive(Debug, Clone)]
pub struct ProbeSessionConfig {
    /// Reference/private verifier maximum exploration depth.
    pub max_depth: usize,
    /// Whether downstream verifiers should write ARD output.
    pub write_ard: bool,
    /// Directory for downstream verifier output.
    pub output_dir: String,
}

impl Default for ProbeSessionConfig {
    fn default() -> Self {
        let max_depth = load_axiom_max_depth()
            .or_else(|| crate::config::load_toml_max_depth())
            .unwrap_or(500);
        Self {
            max_depth,
            write_ard: true,
            output_dir: ".".to_string(),
        }
    }
}

/// Public reference verifier verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceVerdict {
    /// No lock-order cycle was found by the public reference checker.
    Clean,
    /// A lock-order cycle was found.
    BugFound { description: String },
}

/// Result returned by the public reference verifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyResult {
    /// Reference verifier verdict.
    pub verdict: ReferenceVerdict,
    /// Number of public probe events inspected.
    pub events_collected: usize,
}

impl VerifyResult {
    /// Asserts that the public reference verifier found a bug.
    ///
    /// # Panics
    ///
    /// Panics if the verdict is clean.
    pub fn assert_bug(self) {
        match self.verdict {
            ReferenceVerdict::BugFound { description } => {
                println!(
                    "Laplace reference verifier: BUG — {} events ({description})",
                    self.events_collected
                );
            }
            ReferenceVerdict::Clean => {
                panic!("Laplace reference verifier: expected bug but got CLEAN");
            }
        }
    }

    /// Asserts that the public reference verifier found no bug.
    ///
    /// # Panics
    ///
    /// Panics if a lock-order cycle is found.
    pub fn assert_clean(self) {
        match self.verdict {
            ReferenceVerdict::Clean => {
                println!(
                    "Laplace reference verifier: CLEAN — {} events",
                    self.events_collected
                );
            }
            ReferenceVerdict::BugFound { description } => {
                panic!("Laplace reference verifier: bug found: {description}");
            }
        }
    }
}

/// Runs the public reference verifier over collected probe events.
///
/// This does not link any private engine. It performs a conservative lock-order
/// cycle check so public examples can smoke-test instrumentation without Axiom.
#[must_use]
pub fn run_verification_from(
    events: &[ProbeEvent],
    target_name: &str,
    _config: &ProbeSessionConfig,
) -> VerifyResult {
    let verdict = find_lock_order_cycle(events).map_or(ReferenceVerdict::Clean, |cycle| {
        ReferenceVerdict::BugFound {
            description: format!("{target_name}: lock-order cycle {cycle}"),
        }
    });

    VerifyResult {
        verdict,
        events_collected: events.len(),
    }
}

fn find_lock_order_cycle(events: &[ProbeEvent]) -> Option<String> {
    let mut held_by_thread: HashMap<u64, Vec<String>> = HashMap::new();
    let mut order_edges: HashSet<(String, String)> = HashSet::new();

    for event in events {
        match event {
            ProbeEvent::LockAcquired {
                thread_id,
                resource,
            }
            | ProbeEvent::RwLockWriteAcquired {
                thread_id,
                resource,
            } => {
                let held = held_by_thread.entry(*thread_id).or_default();
                for prior in held.iter() {
                    if prior == resource {
                        continue;
                    }
                    if order_edges.contains(&(resource.clone(), prior.clone())) {
                        return Some(format!("{prior}->{resource}->{prior}"));
                    }
                    order_edges.insert((prior.clone(), resource.clone()));
                }
                if !held.contains(resource) {
                    held.push(resource.clone());
                }
            }
            ProbeEvent::LockReleased {
                thread_id,
                resource,
            }
            | ProbeEvent::RwLockWriteReleased {
                thread_id,
                resource,
            } => {
                if let Some(held) = held_by_thread.get_mut(thread_id) {
                    held.retain(|held_resource| held_resource != resource);
                }
            }
            _ => {}
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_get_thread_id() {
        set_probe_thread_id(7);
        assert_eq!(current_thread_id(), 7);
    }

    #[test]
    fn dump_events_writes_envelope_when_env_set_and_noop_otherwise() {
        let events = vec![ProbeEvent::LockAcquired {
            thread_id: 0,
            resource: "a".to_string(),
        }];

        // No env → no-op (must not panic, must not write anywhere).
        std::env::remove_var("LAPLACE_VERIFY_EVENTS_DIR");
        dump_events_if_configured("noop_target", "clean", "fully_deterministic", &events);

        // Env set → writes <target>.json envelope with target/expected/events.
        let dir = std::env::temp_dir().join(format!(
            "laplace-dump-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::env::set_var("LAPLACE_VERIFY_EVENTS_DIR", &dir);
        dump_events_if_configured("my_target", "bug", "best_effort", &events);
        std::env::remove_var("LAPLACE_VERIFY_EVENTS_DIR");

        let path = dir.join("my_target.json");
        let text = std::fs::read_to_string(&path).expect("envelope written");
        let value: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        assert_eq!(value["target"], "my_target");
        assert_eq!(value["expected"], "bug");
        assert_eq!(value["determinism"], "best_effort");
        assert_eq!(value["events"].as_array().map(|a| a.len()), Some(1));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn event_collection_round_trips_through_thread_local_channel() {
        let (tx, rx) = mpsc::sync_channel(16);
        set_probe_sender(tx);
        set_probe_thread_id(0);

        emit(ProbeEvent::LockAcquired {
            thread_id: current_thread_id(),
            resource: "a".to_string(),
        });
        clear_probe_sender();

        let events: Vec<ProbeEvent> = rx.into_iter().collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].resource_name(), Some("a"));
    }

    #[test]
    fn reference_verifier_detects_ab_ba_lock_order_cycle() {
        let events = vec![
            ProbeEvent::LockAcquired {
                thread_id: 0,
                resource: "a".to_string(),
            },
            ProbeEvent::LockAcquired {
                thread_id: 0,
                resource: "b".to_string(),
            },
            ProbeEvent::LockAcquired {
                thread_id: 1,
                resource: "b".to_string(),
            },
            ProbeEvent::LockAcquired {
                thread_id: 1,
                resource: "a".to_string(),
            },
        ];

        let result = run_verification_from(&events, "ab_ba", &ProbeSessionConfig::default());
        assert!(matches!(result.verdict, ReferenceVerdict::BugFound { .. }));
    }
}

#[cfg(feature = "cloud")]
mod cloud {
    use once_cell::sync::OnceCell;

    use crate::client::{ProbeClient, ProbeClientConfig, RawProbeEvent};
    use crate::event::ProbeEvent;

    pub static GLOBAL_PROBE_CLIENT: OnceCell<ProbeClient> = OnceCell::new();

    /// Initializes the global cloud probe transport.
    pub async fn init_cloud_probe(config: ProbeClientConfig) -> anyhow::Result<()> {
        let client = ProbeClient::connect(config).await?;
        let _ = GLOBAL_PROBE_CLIENT.set(client);
        tracing::info!("Laplace cloud probe initialized");
        Ok(())
    }

    /// Converts a public probe event into the RawProbeEvent wire ABI.
    pub fn probe_event_to_raw(event: &ProbeEvent) -> Option<RawProbeEvent> {
        let mut raw: RawProbeEvent = bytemuck::Zeroable::zeroed();
        match event {
            ProbeEvent::LockAcquired { thread_id, .. } => {
                raw.event_type = 4;
                raw.tid = u32::try_from(*thread_id).ok()?;
            }
            ProbeEvent::LockReleased { thread_id, .. } => {
                raw.event_type = 5;
                raw.tid = u32::try_from(*thread_id).ok()?;
            }
            ProbeEvent::ThreadBlocked { thread_id, .. } => {
                raw.event_type = 6;
                raw.tid = u32::try_from(*thread_id).ok()?;
            }
            _ => return None,
        }
        Some(raw)
    }
}

#[cfg(feature = "cloud")]
pub use cloud::init_cloud_probe;
