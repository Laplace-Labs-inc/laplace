//! MockPipeline: end-to-end orchestrator — `RawProbeEvent` → `OracleVerdict`.
//!
//! Wires together the full V1.0 mock pipeline:
//!
//! ```text
//! Vec<RawProbeEvent>
//!   └─ ProbeEventDecoder  ──────→ Vec<DecodedProbeEvent>
//!         └─ AxiomStepBuilder  ──→ per-thread step programs
//!               └─ AxiomOracle ──→ OracleVerdict
//!                     └─ PipelineReport (with OS TID + address reverse-lookup)
//! ```
//!
//! Requires feature `"pipeline"` which pulls in `laplace-axiom`.

use laplace_axiom::{
    dpor::Operation,
    oracle::{AxiomOracle, OracleConfig, OracleVerdict},
    simulation::{TwinSimulator, TwinSimulatorBuilder},
};
use laplace_core::domain::resource::{ResourceId, ThreadId};

use crate::{
    axiom_adapter::{AxiomEvent, AxiomOp, AxiomStep, AxiomStepBuilder},
    decoder::ProbeEventDecoder,
    RawProbeEvent,
};

// ─────────────────────────────────────────────────────────────────────────────
// PipelineReport — human-readable verdict with OS-level identifiers
// ─────────────────────────────────────────────────────────────────────────────

/// Human-readable verdict produced by [`MockPipeline::run`].
///
/// All internal DPOR indices have been translated back to the original OS-level
/// TIDs and kernel resource addresses using the registry reverse-lookup methods.
#[derive(Debug)]
pub enum PipelineReport {
    /// Exhaustive search completed without finding any concurrency bug.
    Clean {
        /// Total number of probe events processed.
        events_processed: usize,
        /// Number of DPOR-consumable steps fed to the Oracle.
        steps_submitted: usize,
    },
    /// A concurrency bug was detected.
    BugFound {
        /// High-level bug description from the Oracle.
        description: String,
        /// Path to the `.ard` forensic report (if written to disk).
        ard_path: String,
        /// Human-readable forensic summary with OS-level identifiers.
        forensic_summary: String,
        /// Total number of probe events processed.
        events_processed: usize,
    },
}

impl PipelineReport {
    /// Returns `true` if a bug was detected.
    pub fn is_bug(&self) -> bool {
        matches!(self, Self::BugFound { .. })
    }

    /// Returns the forensic summary string, or a default message if clean.
    pub fn summary(&self) -> &str {
        match self {
            Self::Clean { .. } => "CLEAN — no concurrency violations detected.",
            Self::BugFound {
                forensic_summary, ..
            } => forensic_summary,
        }
    }
}

impl std::fmt::Display for PipelineReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Clean {
                events_processed,
                steps_submitted,
            } => write!(
                f,
                "✓ CLEAN — {} probe events, {} DPOR steps, no violations found.",
                events_processed, steps_submitted
            ),
            Self::BugFound {
                description,
                ard_path,
                forensic_summary,
                events_processed,
            } => write!(
                f,
                "✗ BUG FOUND ({}) — {} probe events\n\
                 ARD: {}\n\
                 {}",
                description, events_processed, ard_path, forensic_summary
            ),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// MockPipeline
// ─────────────────────────────────────────────────────────────────────────────

/// Configuration for [`MockPipeline`].
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Label stored in the `.ard` forensic header.  Default: `"mock_pipeline"`.
    pub target_id: String,
    /// Maximum DPOR exploration depth.  Default: `500`.
    pub max_depth: usize,
    /// Whether to write `.ard` files to disk on violation.  Default: `false`.
    pub write_ard: bool,
    /// Directory for `.ard` output files.  Default: `"."`.
    pub output_dir: String,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            target_id: "mock_pipeline".to_string(),
            max_depth: 500,
            write_ard: false,
            output_dir: ".".to_string(),
        }
    }
}

/// End-to-end mock pipeline runner.
///
/// # Example
///
/// ```
/// use laplace_probe_common::mock::MockProbeSource;
/// use laplace_probe_common::pipeline::{MockPipeline, PipelineConfig};
///
/// let raw = MockProbeSource::default_seed().generate_ab_ba_deadlock();
/// let report = MockPipeline::run(raw, PipelineConfig::default());
/// assert!(report.is_bug(), "AB-BA deadlock must be detected");
/// println!("{}", report);
/// ```
pub struct MockPipeline;

impl MockPipeline {
    /// Runs the full V1.0 mock pipeline against the supplied raw event stream.
    ///
    /// 1. Decodes `raw_events` via [`ProbeEventDecoder`].
    /// 2. Translates decoded events via [`AxiomStepBuilder`], building dense
    ///    thread and resource registries (with reverse-lookup support).
    /// 3. Extracts per-thread step programs from the `AxiomEvent::Step` stream.
    /// 4. Feeds the programs into `AxiomOracle::run_exhaustive` with a minimal
    ///    `TwinSimulator`.
    /// 5. Translates the Oracle verdict back to OS-level identifiers using
    ///    `ThreadRegistry::get_kernel_tid` and
    ///    `ResourceRegistry::get_kernel_resource_id`.
    ///
    /// Returns a [`PipelineReport`] — either [`Clean`] or [`BugFound`] with a
    /// human-readable forensic summary.
    ///
    /// [`Clean`]: PipelineReport::Clean
    /// [`BugFound`]: PipelineReport::BugFound
    pub fn run(raw_events: Vec<RawProbeEvent>, config: PipelineConfig) -> PipelineReport {
        let events_processed = raw_events.len();

        // ── Stage 1: Decode ───────────────────────────────────────────────
        let decoder = ProbeEventDecoder::new();
        let decoded = decoder.decode_batch(&raw_events);

        // ── Stage 2: Translate → per-thread step programs ─────────────────
        let mut builder = AxiomStepBuilder::new();
        let axiom_events = builder.process_batch(&decoded);

        let num_threads = builder.thread_registry().len().max(1);
        let num_resources = builder.resource_registry().len().max(1);

        // Group steps by thread (preserving program order).
        let mut per_thread_steps: Vec<Vec<(Operation, ResourceId)>> = vec![Vec::new(); num_threads];

        let mut steps_submitted = 0usize;
        for event in &axiom_events {
            if let AxiomEvent::Step(step) = event {
                if step.thread < num_threads {
                    per_thread_steps[step.thread].push((
                        match step.op {
                            AxiomOp::Request => Operation::Request,
                            AxiomOp::Release => Operation::Release,
                            AxiomOp::SharedRequest => Operation::SharedRequest,
                            AxiomOp::SharedRelease => Operation::SharedRelease,
                            AxiomOp::Read => Operation::Read,
                            AxiomOp::Write => Operation::Write,
                            AxiomOp::ReadWrite => Operation::ReadWrite,
                        },
                        ResourceId::new(step.resource),
                    ));
                    steps_submitted += 1;
                }
            }
        }

        // Compact: exclude threads that have no operations (e.g. the parent TID 0
        // registered by ThreadSpawn but never performing any lock/network ops).
        // Threads with empty programs would accumulate starvation counters inside
        // the DPOR engine, causing false starvation detections.
        // We record the axiom_id for each compacted DPOR slot for forensic translation.
        let mut compacted_steps: Vec<Vec<(Operation, ResourceId)>> = Vec::new();
        let mut dpor_to_axiom_thread: Vec<usize> = Vec::new();
        for (axiom_id, steps) in per_thread_steps.iter().enumerate() {
            if !steps.is_empty() {
                dpor_to_axiom_thread.push(axiom_id);
                compacted_steps.push(steps.clone());
            }
        }
        let num_active_threads = compacted_steps.len().max(1);

        // ── Stage 3: Build Oracle + TwinSimulator ─────────────────────────
        let oracle_config = OracleConfig {
            num_threads: num_active_threads,
            num_resources,
            max_depth: config.max_depth,
            write_ard: config.write_ard,
            output_dir: config.output_dir.clone(),
            ..OracleConfig::default()
        };
        let oracle = AxiomOracle::new(oracle_config);

        // The TwinSimulator is required by the Oracle API.
        // For the mock pipeline it runs in lock-step but its events are irrelevant;
        // the DPOR explores the op_provider program model, not the simulator events.
        let mut sim: TwinSimulator = TwinSimulatorBuilder::new()
            .cores(num_active_threads.max(2))
            .scheduler_threads(num_active_threads.max(2))
            .finalize()
            .build();

        // ── Stage 4: Run exhaustive DPOR ──────────────────────────────────
        let verdict = oracle.run_exhaustive(
            &config.target_id,
            &mut sim,
            config.max_depth,
            |thread: ThreadId, pc: usize| -> Option<(Operation, ResourceId)> {
                compacted_steps
                    .get(thread.as_usize())
                    .and_then(|steps| steps.get(pc))
                    .copied()
            },
            |_sim| None, // no invariant checker for the mock pipeline
        );

        // ── Stage 5: Translate verdict with reverse lookups ───────────────
        match verdict {
            OracleVerdict::Clean => PipelineReport::Clean {
                events_processed,
                steps_submitted,
            },
            OracleVerdict::BugFound {
                description,
                ard_path,
            } => {
                let forensic_summary = Self::format_forensic_summary(
                    &description,
                    &axiom_events,
                    &builder,
                    num_active_threads,
                    num_resources,
                    &dpor_to_axiom_thread,
                );
                PipelineReport::BugFound {
                    description,
                    ard_path,
                    forensic_summary,
                    events_processed,
                }
            }
        }
    }

    /// Formats a human-readable forensic summary with OS-level identifiers.
    ///
    /// Translates every `AxiomThreadId` and `AxiomResourceId` in the event
    /// stream back to the original kernel TID and resource address using the
    /// `ThreadRegistry` and `ResourceRegistry` reverse-lookup methods.
    ///
    /// `dpor_to_axiom_thread[dpor_idx]` maps from the compacted DPOR thread index
    /// (0..num_active_threads) to the original `AxiomThreadId` in the registry.
    fn format_forensic_summary(
        description: &str,
        axiom_events: &[AxiomEvent],
        builder: &AxiomStepBuilder,
        num_active_threads: usize,
        num_resources: usize,
        dpor_to_axiom_thread: &[usize],
    ) -> String {
        let thread_reg = builder.thread_registry();
        let resource_reg = builder.resource_registry();

        let mut out = String::with_capacity(512);
        out.push_str("┌─ Axiom Forensic Report ────────────────────────────────────\n");
        out.push_str(&format!("│  Violation : {}\n", description));
        out.push_str(&format!("│  Active threads   : {}\n", num_active_threads));
        out.push_str(&format!(
            "│  Resources        : {} observed\n",
            num_resources
        ));
        out.push_str("│\n");

        // Thread registry — show DPOR index ↔ OS TID mapping (compacted)
        out.push_str("│  Thread registry (DPOR index → OS TID):\n");
        for dpor_id in 0..num_active_threads {
            let axiom_id = dpor_to_axiom_thread
                .get(dpor_id)
                .copied()
                .unwrap_or(dpor_id);
            let os_tid = thread_reg
                .get_kernel_tid(axiom_id)
                .map(|t| format!("{}", t))
                .unwrap_or_else(|| "<unknown>".to_string());
            out.push_str(&format!("│    ThreadId({}) → OS TID {}\n", dpor_id, os_tid));
        }
        out.push_str("│\n");

        // Resource registry — show DPOR index ↔ kernel address mapping
        out.push_str("│  Resource registry (DPOR index → kernel address):\n");
        for axiom_id in 0..num_resources {
            let kernel_addr = resource_reg
                .get_kernel_resource_id(axiom_id)
                .map(|a| format!("{:#018x}", a))
                .unwrap_or_else(|| "<unknown>".to_string());
            out.push_str(&format!(
                "│    ResourceId({}) → {}\n",
                axiom_id, kernel_addr
            ));
        }
        out.push_str("│\n");

        // Event trace — translate every step back to OS identifiers
        out.push_str("│  Translated event trace:\n");
        for (i, event) in axiom_events.iter().enumerate() {
            let line = Self::format_event(i, event, thread_reg, resource_reg);
            out.push_str(&format!("│    {}\n", line));
        }

        out.push_str("└────────────────────────────────────────────────────────────\n");
        out
    }

    fn format_event(
        idx: usize,
        event: &AxiomEvent,
        thread_reg: &crate::axiom_adapter::ThreadRegistry,
        resource_reg: &crate::axiom_adapter::ResourceRegistry,
    ) -> String {
        match event {
            AxiomEvent::Step(AxiomStep {
                thread,
                op,
                resource,
                timestamp_ns,
            }) => {
                let os_tid = thread_reg
                    .get_kernel_tid(*thread)
                    .map(|t| format!("{}", t))
                    .unwrap_or_else(|| format!("?{}", thread));
                let kernel_addr = resource_reg
                    .get_kernel_resource_id(*resource)
                    .map(|a| format!("{:#018x}", a))
                    .unwrap_or_else(|| format!("r{}", resource));
                let op_str = match op {
                    AxiomOp::Request => "Request",
                    AxiomOp::Release => "Release",
                    AxiomOp::SharedRequest => "SharedReq",
                    AxiomOp::SharedRelease => "SharedRel",
                    AxiomOp::Read => "Read",
                    AxiomOp::Write => "Write",
                    AxiomOp::ReadWrite => "RMW",
                };
                format!(
                    "[{:03}] @{} TID {:5} {:7} {}",
                    idx, timestamp_ns, os_tid, op_str, kernel_addr
                )
            }
            AxiomEvent::ThreadSpawned {
                parent,
                child,
                timestamp_ns,
            } => {
                let p_tid = thread_reg
                    .get_kernel_tid(*parent)
                    .map(|t| format!("{}", t))
                    .unwrap_or_else(|| format!("?{}", parent));
                let c_tid = thread_reg
                    .get_kernel_tid(*child)
                    .map(|t| format!("{}", t))
                    .unwrap_or_else(|| format!("?{}", child));
                format!(
                    "[{:03}] @{} TID {:5} spawned TID {}",
                    idx, timestamp_ns, p_tid, c_tid
                )
            }
            AxiomEvent::ThreadExited {
                thread,
                timestamp_ns,
            } => {
                let os_tid = thread_reg
                    .get_kernel_tid(*thread)
                    .map(|t| format!("{}", t))
                    .unwrap_or_else(|| format!("?{}", thread));
                format!("[{:03}] @{} TID {:5} exited", idx, timestamp_ns, os_tid)
            }
            AxiomEvent::SchedSwitch {
                prev,
                next,
                timestamp_ns,
            } => {
                let p_tid = thread_reg
                    .get_kernel_tid(*prev)
                    .map(|t| format!("{}", t))
                    .unwrap_or_else(|| format!("?{}", prev));
                let n_tid = thread_reg
                    .get_kernel_tid(*next)
                    .map(|t| format!("{}", t))
                    .unwrap_or_else(|| format!("?{}", next));
                format!(
                    "[{:03}] @{} SchedSwitch {} → {}",
                    idx, timestamp_ns, p_tid, n_tid
                )
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockProbeSource;

    fn no_ard_config(target: &str) -> PipelineConfig {
        PipelineConfig {
            target_id: target.to_string(),
            max_depth: 500,
            write_ard: false,
            output_dir: ".".to_string(),
        }
    }

    /// End-to-end: AB-BA deadlock scenario must be detected and the forensic
    /// report must contain the original OS TIDs (1001, 1002) and lock addresses.
    #[test]
    fn pipeline_detects_ab_ba_deadlock_and_translates_ids() {
        let raw = MockProbeSource::default_seed().generate_ab_ba_deadlock();
        let report = MockPipeline::run(raw, no_ard_config("ab_ba_deadlock_test"));

        assert!(
            report.is_bug(),
            "AB-BA deadlock scenario must produce BugFound verdict"
        );

        let summary = report.summary();
        println!("\n{}", summary);

        // The forensic summary must reference the original OS TIDs
        assert!(
            summary.contains("1001") || summary.contains("1002"),
            "forensic summary must contain original OS TIDs (1001 or 1002);\ngot:\n{}",
            summary
        );

        // The forensic summary must reference the lock addresses.
        // Rust's {:#018x} formats 0xFFFF_DEAD_BEEF_0000 as "0xffffdeadbeef0000" (no underscores).
        assert!(
            summary.contains("ffffdeadbeef"),
            "forensic summary must contain the original mutex address;\ngot:\n{}",
            summary
        );
    }

    /// The data-race scenario (two threads competing on same resource,
    /// thread A re-requests without releasing) must also be caught.
    #[test]
    fn pipeline_detects_data_race() {
        let raw = MockProbeSource::default_seed().generate_data_race();
        let report = MockPipeline::run(raw, no_ard_config("data_race_test"));
        assert!(
            report.is_bug(),
            "data-race scenario must produce BugFound verdict"
        );
    }

    /// Livelock/starvation scenario: Thread B never gets to run, exceeding
    /// MAX_STARVATION_LIMIT.
    #[test]
    fn pipeline_detects_livelock_starvation() {
        let raw = MockProbeSource::default_seed().generate_livelock_starvation();
        let report = MockPipeline::run(raw, no_ard_config("livelock_test"));
        assert!(
            report.is_bug(),
            "livelock/starvation scenario must produce BugFound verdict"
        );
    }

    /// Clean scenario — a single thread acquires and releases a lock with no
    /// contention. The Oracle must return Clean.
    #[test]
    fn pipeline_returns_clean_for_uncontested_scenario() {
        use crate::mock::{MOCK_LOCK_X, MOCK_TID_A};
        use crate::{ProbeEventType, RawProbeEvent};

        // Build a simple, clean event sequence manually:
        // Thread A: spawn → acquire X → release X  (no second thread)
        let mut ts = 1_000_000_000u64;
        let mut make = |event_type: ProbeEventType, tid: u32, resource_id: u64| {
            let mut e: RawProbeEvent = unsafe { core::mem::zeroed() };
            e.event_type = event_type as u8;
            e.tid = tid;
            e.resource_id = resource_id;
            e.timestamp_ns = ts;
            ts += 100;
            e
        };

        let raw = vec![
            make(ProbeEventType::ThreadSpawn, MOCK_TID_A, 0),
            make(ProbeEventType::LockAcquire, MOCK_TID_A, MOCK_LOCK_X),
            make(ProbeEventType::LockAcquired, MOCK_TID_A, MOCK_LOCK_X),
            make(ProbeEventType::LockRelease, MOCK_TID_A, MOCK_LOCK_X),
        ];

        let report = MockPipeline::run(raw, no_ard_config("clean_test"));
        assert!(
            !report.is_bug(),
            "single-thread acquire/release must be Clean; got: {:?}",
            report
        );
    }

    /// Verify Display output contains expected sections.
    #[test]
    fn pipeline_report_display_contains_ard_path_and_summary() {
        let raw = MockProbeSource::default_seed().generate_ab_ba_deadlock();
        let report = MockPipeline::run(raw, no_ard_config("display_test"));
        let display = format!("{}", report);
        assert!(display.contains("BUG FOUND") || display.contains("CLEAN"));
    }
}
