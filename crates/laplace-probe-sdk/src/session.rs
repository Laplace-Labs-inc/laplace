// SPDX-License-Identifier: Apache-2.0
//! Public in-memory probe event collection.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Mutex as StdMutex, MutexGuard, OnceLock, PoisonError};
use std::thread::JoinHandle;

use crate::event::ProbeEvent;
use crate::license::load_axiom_max_depth;

thread_local! {
    /// Current OS-thread event sink. Threads without a registered sink drop events.
    static PROBE_SENDER: std::cell::RefCell<Option<mpsc::SyncSender<ProbeEvent>>> =
        const { std::cell::RefCell::new(None) };

    /// Current OS-thread logical thread id used by generated test harnesses.
    static PROBE_THREAD_ID: Cell<Option<u64>> = const { Cell::new(None) };

    /// Current task being polled on this OS thread. Unlike `PROBE_THREAD_ID`,
    /// this is cleared at the poll boundary so spawn callbacks outside a poll
    /// cannot inherit a stale parent.
    static CURRENT_TASK_ID: Cell<Option<u64>> = const { Cell::new(None) };
}

static GLOBAL_PROBE_SENDER: OnceLock<StdMutex<Option<mpsc::SyncSender<ProbeEvent>>>> =
    OnceLock::new();
static NEXT_IMPLICIT_THREAD_ID: AtomicU64 = AtomicU64::new(0);

/// Active [`CaptureSession`] event sink (unbounded — never blocks a sender, so a
/// producer that outruns the collector cannot deadlock a joining harness).
///
/// This is the sink used by generated `#[laplace::verify]` harnesses: worker and
/// model-spawned child threads emit here without per-thread registration, and the
/// owning session drains it concurrently. Distinct from the legacy
/// [`set_probe_sender`] thread-local/global path so hand-written examples keep
/// working unchanged.
static SESSION_SENDER: OnceLock<StdMutex<Option<mpsc::Sender<ProbeEvent>>>> = OnceLock::new();

/// Process-wide capture exclusivity. Only one [`CaptureSession`] is active at a
/// time so parallel generated tests in one binary cannot cross-pollute the shared
/// [`SESSION_SENDER`] slot or the implicit thread-id counter.
static CAPTURE_LOCK: StdMutex<()> = StdMutex::new(());

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

/// A scoped event-capture session for generated verification harnesses.
///
/// `begin` installs the process-wide session sink and starts a background
/// collector that drains events *concurrently* with the run, then `finish`
/// tears the sink down and returns every captured event in order. This replaces
/// the earlier "register a global sender, then drain the channel after joining"
/// harness shape, which had three defects a generated test could not avoid:
///
/// 1. **Hang** — a process-global sender clone was never cleared, so the
///    post-join `rx.into_iter()` waited forever for a sender that never dropped.
/// 2. **Buffer deadlock** — a bounded channel drained only after `join` would
///    block a producing thread (hence `join`) once event count exceeded the
///    buffer. The session sink is unbounded and drained live, so producers
///    never block.
/// 3. **Parallel cross-pollution** — two generated tests in one binary shared
///    the single global slot and implicit-id counter. [`CAPTURE_LOCK`] makes a
///    session exclusive for its lifetime, so parallel tests serialize cleanly.
///
/// The session is RAII: dropping it without calling [`finish`](Self::finish)
/// still tears the sink down and joins the collector, so a panicking harness
/// cannot leave a dangling global sink for the next test.
///
/// > [GHOST_CONSTRAINT: target=CaptureSession] Exactly one session may be active
/// > per process; `begin` blocks until any prior session finishes. Events emitted
/// > with no active session (and no legacy sender) are dropped, never buffered.
#[must_use = "a CaptureSession must be finished to collect events and release the capture lock"]
pub struct CaptureSession {
    _guard: MutexGuard<'static, ()>,
    collector: Option<JoinHandle<Vec<ProbeEvent>>>,
}

impl CaptureSession {
    /// Begins an exclusive capture session.
    ///
    /// Blocks until any currently active session finishes, then installs the
    /// unbounded session sink, resets implicit thread-id allocation, and starts
    /// the background collector.
    pub fn begin() -> Self {
        let guard = CAPTURE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        NEXT_IMPLICIT_THREAD_ID.store(0, Ordering::SeqCst);

        let (tx, rx) = mpsc::channel::<ProbeEvent>();
        let slot = SESSION_SENDER.get_or_init(|| StdMutex::new(None));
        *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(tx);

        let collector = std::thread::Builder::new()
            .name("laplace-capture-collector".to_string())
            .spawn(move || rx.into_iter().collect::<Vec<ProbeEvent>>())
            .expect("laplace: failed to spawn capture collector thread");

        Self {
            _guard: guard,
            collector: Some(collector),
        }
    }

    /// Finishes the session and returns every captured event, in emission order.
    #[must_use]
    pub fn finish(mut self) -> Vec<ProbeEvent> {
        self.tear_down()
    }

    /// Clears the session sink (dropping the sole sender so the collector's
    /// `rx` closes) and joins the collector.
    fn tear_down(&mut self) -> Vec<ProbeEvent> {
        if let Some(slot) = SESSION_SENDER.get() {
            *slot.lock().unwrap_or_else(PoisonError::into_inner) = None;
        }
        self.collector
            .take()
            .map(|collector| collector.join().unwrap_or_default())
            .unwrap_or_default()
    }
}

impl Drop for CaptureSession {
    fn drop(&mut self) {
        if self.collector.is_some() {
            let _ = self.tear_down();
        }
    }
}

/// Assigns the current OS thread's logical probe thread id.
pub fn set_probe_thread_id(id: u64) {
    PROBE_THREAD_ID.with(|c| c.set(Some(id)));
}

pub(crate) fn set_current_task_id(id: u64) {
    CURRENT_TASK_ID.with(|c| c.set(Some(id)));
}

pub(crate) fn clear_current_task_id() {
    CURRENT_TASK_ID.with(|c| c.set(None));
}

pub(crate) fn current_task_id() -> Option<u64> {
    CURRENT_TASK_ID.with(Cell::get)
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

/// Emits a public probe event to the active sink.
///
/// Priority: the OS-thread-local sink (legacy hand-written harnesses), then the
/// legacy process-global sink, then the active [`CaptureSession`] sink. A
/// generated `#[laplace::verify]` harness registers none of the legacy sinks, so
/// its worker and model-spawned child threads land on the session sink.
pub fn emit(event: ProbeEvent) {
    let emitted_to_thread_local = PROBE_SENDER.with(|s| {
        if let Some(tx) = s.borrow().as_ref() {
            let _ = tx.send(event.clone());
            return true;
        }
        false
    });

    if !emitted_to_thread_local {
        let emitted_to_global = GLOBAL_PROBE_SENDER.get().is_some_and(|slot| {
            if let Some(tx) = slot.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
                let _ = tx.send(event.clone());
                return true;
            }
            false
        });

        if !emitted_to_global {
            if let Some(slot) = SESSION_SENDER.get() {
                if let Some(tx) = slot.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
                    let _ = tx.send(event.clone());
                }
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
/// ingestion contract for the private CLI/API verdict boundary. Only the file
/// name is sanitized (`::` → `__`); the envelope `target` field preserves the
/// full target name.
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
/// `events` may carry the [v2] async vocabulary (`TaskSpawned`/`TaskPolled`/…)
/// alongside the v1 thread/resource events; the envelope declares
/// `"schema_version": "2"` unconditionally since this producer version, so a
/// consumer can tell an async-capable dump from a pre-v2 one without sniffing
/// the event list. This is additive: a consumer that only understands v1
/// events keeps parsing the `events` array unchanged and simply observes the
/// new field.
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
    dump_events_with_mode(target_name, expected, determinism, "verify", events);
}

/// [`dump_events_if_configured`] with an explicit capture mode.
///
/// `capture_mode` records how the capture was composed: `"verify"` for the
/// exploration harness modes (`threads`/`tasks`), `"scenario"` for a one-shot
/// representative execution. Additive envelope field: consumers surface
/// `"scenario"` so a single-run capture is never mistaken for an exploration
/// of the target's schedules; older consumers keep parsing unchanged.
pub fn dump_events_with_mode(
    target_name: &str,
    expected: &str,
    determinism: &str,
    capture_mode: &str,
    events: &[ProbeEvent],
) {
    let Ok(dir) = std::env::var("LAPLACE_VERIFY_EVENTS_DIR") else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let filename = format!("{}.json", target_name.replace("::", "__"));
    let path = std::path::Path::new(&dir).join(filename);
    let envelope = serde_json::json!({
        "schema_version": "2",
        "target": target_name,
        "expected": expected,
        "determinism": determinism,
        "capture_mode": capture_mode,
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
            .or_else(|| warn_toml_depth_override(crate::config::load_toml_max_depth()))
            .unwrap_or(500);
        Self {
            max_depth,
            write_ard: true,
            output_dir: ".".to_string(),
        }
    }
}

fn warn_toml_depth_override(depth: Option<usize>) -> Option<usize> {
    depth.inspect(|depth| {
        tracing::warn!(
            "axiom exploration depth set to {depth} by a discovered laplace.toml \
             ([axiom] max_depth); this silently overrides the built-in default. \
             Set the depth via your Laplace license (~/.laplace/config.json) or \
             remove the laplace.toml entry to make verification depth explicit."
        );
    })
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

/// Hold/acquire mode for lock-order-cycle detection. Shared (RwLock read)
/// holds participate in deadlock cycles — a reader blocks a writer — but two
/// shared operations on the same resource never conflict with each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum HoldMode {
    Exclusive,
    Shared,
}

fn modes_conflict(a: HoldMode, b: HoldMode) -> bool {
    !(a == HoldMode::Shared && b == HoldMode::Shared)
}

fn find_lock_order_cycle(events: &[ProbeEvent]) -> Option<String> {
    let mut held_by_thread: HashMap<u64, Vec<(String, HoldMode)>> = HashMap::new();
    // Edge (held, held_mode, acquired, acquired_mode): some thread acquired
    // `acquired` while holding `held`. A reverse edge closes a cycle only when
    // both wait directions are mode-conflicting (read-read cannot block).
    let mut order_edges: HashSet<(String, HoldMode, String, HoldMode)> = HashSet::new();

    for event in events {
        let (thread_id, resource, mode, is_acquire) = match event {
            ProbeEvent::LockAcquired {
                thread_id,
                resource,
            } => (thread_id, resource, HoldMode::Exclusive, true),
            ProbeEvent::RwLockWriteAcquired {
                thread_id,
                resource,
            } => (thread_id, resource, HoldMode::Exclusive, true),
            ProbeEvent::RwLockReadAcquired {
                thread_id,
                resource,
            } => (thread_id, resource, HoldMode::Shared, true),
            ProbeEvent::LockReleased {
                thread_id,
                resource,
            }
            | ProbeEvent::RwLockWriteReleased {
                thread_id,
                resource,
            }
            | ProbeEvent::RwLockReadReleased {
                thread_id,
                resource,
            } => (thread_id, resource, HoldMode::Exclusive, false),
            _ => continue,
        };

        if is_acquire {
            let held = held_by_thread.entry(*thread_id).or_default();
            for (prior, prior_mode) in held.iter() {
                if prior == resource {
                    continue;
                }
                // Reverse edge: another interleaving holds `resource` (their_hold)
                // and wants `prior` (their_want). Deadlock needs both waits to
                // block: our acquire vs their hold, and their want vs our hold.
                let cycle = order_edges.iter().any(|(r, their_hold, p, their_want)| {
                    r == resource
                        && p == prior
                        && modes_conflict(mode, *their_hold)
                        && modes_conflict(*their_want, *prior_mode)
                });
                if cycle {
                    return Some(format!("{prior}->{resource}->{prior}"));
                }
                order_edges.insert((prior.clone(), *prior_mode, resource.clone(), mode));
            }
            if !held
                .iter()
                .any(|(held_resource, _)| held_resource == resource)
            {
                held.push((resource.clone(), mode));
            }
        } else if let Some(held) = held_by_thread.get_mut(thread_id) {
            held.retain(|(held_resource, _)| held_resource != resource);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{BroadcastOp, BroadcastOutcome};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    struct EventCounter(Arc<AtomicUsize>);

    impl tracing::Subscriber for EventCounter {
        fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }

        fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}

        fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}

        fn event(&self, _event: &tracing::Event<'_>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }

        fn enter(&self, _span: &tracing::span::Id) {}

        fn exit(&self, _span: &tracing::span::Id) {}
    }

    #[test]
    fn toml_depth_override_warns_and_preserves_value() {
        let events = Arc::new(AtomicUsize::new(0));
        let result = tracing::subscriber::with_default(EventCounter(Arc::clone(&events)), || {
            warn_toml_depth_override(Some(20))
        });

        assert_eq!(result, Some(20));
        assert_eq!(events.load(Ordering::SeqCst), 1);
        assert_eq!(warn_toml_depth_override(None), None);
    }

    #[test]
    fn set_and_get_thread_id() {
        set_probe_thread_id(7);
        assert_eq!(current_thread_id(), 7);
    }

    /// `LAPLACE_VERIFY_EVENTS_DIR` is process-global; dump tests must not
    /// interleave their set/remove windows.
    static DUMP_ENV_LOCK: StdMutex<()> = StdMutex::new(());

    #[test]
    fn dump_events_writes_envelope_when_env_set_and_noop_otherwise() {
        let _env = DUMP_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
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
        dump_events_if_configured("my_crate::module::my_target", "bug", "declared", &events);
        dump_events_with_mode(
            "my_crate::module::my_scenario",
            "clean",
            "fully_deterministic",
            "scenario",
            &events,
        );
        std::env::remove_var("LAPLACE_VERIFY_EVENTS_DIR");

        let path = dir.join("my_crate__module__my_target.json");
        let text = std::fs::read_to_string(&path).expect("envelope written");
        let value: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        assert_eq!(value["schema_version"], "2");
        assert_eq!(value["target"], "my_crate::module::my_target");
        assert_eq!(value["expected"], "bug");
        assert_eq!(value["determinism"], "declared");
        assert_eq!(value["capture_mode"], "verify");
        assert_eq!(value["events"].as_array().map(|a| a.len()), Some(1));

        let scenario_path = dir.join("my_crate__module__my_scenario.json");
        let scenario_text = std::fs::read_to_string(&scenario_path).expect("scenario written");
        let scenario_value: serde_json::Value =
            serde_json::from_str(&scenario_text).expect("valid json");
        assert_eq!(scenario_value["capture_mode"], "scenario");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dump_events_round_trips_async_variants_under_schema_version_2() {
        let _env = DUMP_ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let events = vec![
            ProbeEvent::TaskSpawned {
                task_id: 1,
                parent_task_id: None,
                source_location: Some("src/main.rs:10".to_string()),
            },
            ProbeEvent::TaskPolled {
                task_id: 1,
                poll_attempt_id: 0,
            },
            ProbeEvent::FuturePending {
                task_id: 1,
                future_id: Some(2),
                poll_attempt_id: 0,
            },
            ProbeEvent::WakeIssued {
                source_task_id: None,
                target_task_id: 1,
                waker_id: 9,
            },
            ProbeEvent::FutureReady {
                task_id: 1,
                future_id: Some(2),
                poll_attempt_id: 1,
            },
            ProbeEvent::TaskCompleted { task_id: 1 },
            ProbeEvent::CancelRequested { task_id: 2 },
            ProbeEvent::AsyncBroadcastOpResolved {
                thread_id: 0,
                resource: 4,
                op: 5,
                receiver_id: Some(6),
                op_kind: BroadcastOp::Recv,
                outcome: BroadcastOutcome::Lagged { missed: 7 },
            },
            ProbeEvent::LockAcquired {
                thread_id: 0,
                resource: "a".to_string(),
            },
        ];

        let dir = std::env::temp_dir().join(format!(
            "laplace-dump-v2-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::env::set_var("LAPLACE_VERIFY_EVENTS_DIR", &dir);
        dump_events_if_configured("async_target", "clean", "fully_deterministic", &events);
        std::env::remove_var("LAPLACE_VERIFY_EVENTS_DIR");

        let path = dir.join("async_target.json");
        let text = std::fs::read_to_string(&path).expect("envelope written");
        let value: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        assert_eq!(value["schema_version"], "2");

        // Round-trip the events array back through the real enum, not just JSON.
        let round_tripped: Vec<ProbeEvent> =
            serde_json::from_value(value["events"].clone()).expect("events round-trip");
        assert_eq!(round_tripped.len(), events.len());
        assert!(matches!(round_tripped[0], ProbeEvent::TaskSpawned { .. }));
        assert!(matches!(
            round_tripped[7],
            ProbeEvent::AsyncBroadcastOpResolved {
                outcome: BroadcastOutcome::Lagged { missed: 7 },
                ..
            }
        ));
        assert!(matches!(round_tripped[8], ProbeEvent::LockAcquired { .. }));

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
    fn capture_session_begin_finish_is_exclusive_and_returns_empty_when_idle() {
        // A session with no emitters drains to empty (collector joins cleanly);
        // exercising begin→finish twill in sequence proves CAPTURE_LOCK releases
        // on finish so a second session is not deadlocked by the first.
        let events = CaptureSession::begin().finish();
        assert!(events.is_empty());
        let again = CaptureSession::begin().finish();
        assert!(again.is_empty());
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
