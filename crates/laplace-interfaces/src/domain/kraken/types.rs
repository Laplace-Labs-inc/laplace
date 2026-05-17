//! # Kraken DSL Domain Types
//!
//! All public data types for the Kraken load-testing and digital-twin DSL.
//! These types define the contract between the Kraken scenario engine (producer)
//! and any downstream consumer (executor, reporter, TUI).
//!
//! ## Types
//!
//! - [`VUState`] — Virtual User state machine states
//! - [`ThinkTimeDistribution`] — Think time distribution parameters
//! - [`ScenarioStep`] — Individual instruction in a VU execution script
//! - [`Scenario`] — Immutable execution program for VUs
//! - [`ChaosEvent`] — Chaos event types for deterministic failure injection
//! - [`ChaosSchedule`] — Collection of chaos events with efficient lookup
//! - [`RampUpProfile`] — Load increase patterns for virtual user injection

#![cfg(feature = "twin")]

use crate::domain::transport::HttpMethod;
use crate::error::kraken::{KrakenError, Result as KrakenResult};
use serde::{Deserialize, Serialize};
use std::fmt;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

// ─── VUState ──────────────────────────────────────────────────────────────────

/// Virtual User State Machine States
///
/// # TLA+ Correspondence
/// Corresponds to `VUState` in KrakenAnatomy.tla:
/// ```tla
/// VUState == {"Idle", "Thinking", "Requesting", "Validating", "Waiting", "Finished"}
/// ```
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Kraken",
        link = "LEP-0012-laplace-interfaces-kraken_deterministic_chaos"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VUState {
    /// Initial state, awaiting spawn action (TLA+: `"Idle"`)
    Idle,
    /// Active computational phase, planning next action (TLA+: `"Thinking"`)
    Thinking,
    /// Sending request to external service (TLA+: `"Requesting"`)
    Requesting,
    /// Validating response correctness (TLA+: `"Validating"`)
    Validating,
    /// Idle wait for timeout or external event (TLA+: `"Waiting"`)
    Waiting,
    /// Terminal state, VU has completed (TLA+: `"Finished"`)
    Finished,
}

impl VUState {
    /// Returns `true` if this is the terminal `Finished` state
    pub fn is_terminal(&self) -> bool {
        matches!(self, VUState::Finished)
    }

    /// Returns `true` for states where the scheduler must not run this VU
    ///
    /// Blocking states: `Requesting`, `Waiting`
    pub fn is_blocking(&self) -> bool {
        matches!(self, VUState::Requesting | VUState::Waiting)
    }
}

impl fmt::Display for VUState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => write!(f, "Idle"),
            Self::Thinking => write!(f, "Thinking"),
            Self::Requesting => write!(f, "Requesting"),
            Self::Validating => write!(f, "Validating"),
            Self::Waiting => write!(f, "Waiting"),
            Self::Finished => write!(f, "Finished"),
        }
    }
}

// ─── ThinkTimeDistribution ────────────────────────────────────────────────────

/// Think Time Distribution — controls how VU idle durations are sampled
///
/// Used as the parameter type for [`ScenarioStep::Think`].
/// All values are expressed in milliseconds.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Kraken",
        link = "LEP-0012-laplace-interfaces-kraken_deterministic_chaos"
    )
)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum ThinkTimeDistribution {
    /// Gaussian distribution (Box-Muller transform via deterministic RNG).
    ///
    /// Values are clamped to `≥ 0` so negative durations never occur.
    Normal {
        /// Mean duration in milliseconds
        mean: f64,
        /// Standard deviation in milliseconds
        std_dev: f64,
    },
    /// Uniform distribution in `[min, max]` (inclusive, milliseconds).
    Uniform {
        /// Minimum duration in milliseconds
        min: u64,
        /// Maximum duration in milliseconds
        max: u64,
    },
}

impl fmt::Display for ThinkTimeDistribution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Normal { mean, std_dev } => {
                write!(f, "Normal(μ={mean:.0}ms, σ={std_dev:.0}ms)")
            }
            Self::Uniform { min, max } => write!(f, "Uniform({min}ms..{max}ms)"),
        }
    }
}

// ─── ScenarioStep ─────────────────────────────────────────────────────────────

/// Scenario Step — individual instruction in a VU execution script
///
/// Models instruction pointer sequences in KrakenAnatomy.tla:
/// ```tla
/// InstructionType == {"Think", "Request", "Branch", "RetryOnError", "Loop", "Finish"}
/// ```
///
/// All transitions are deterministic given the current step, VU's RNG state,
/// and the simulation tick counter.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Kraken",
        link = "LEP-0012-laplace-interfaces-kraken_deterministic_chaos"
    )
)]
#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum ScenarioStep {
    /// Think/Wait Step: deterministic idle using a [`ThinkTimeDistribution`].
    ///
    /// Samples a duration, sets `context.wait_until`, advances PC, and yields.
    Think {
        /// Distribution for sampling think time duration
        distribution: ThinkTimeDistribution,
    },

    /// Request Step: send an HTTP request through the virtual transport.
    ///
    /// Path and body support `{{vu_id}}` / `{{key}}` placeholder substitution.
    Request {
        /// HTTP verb (GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS)
        method: HttpMethod,
        /// Target URL path template
        path: &'static str,
        /// Optional request body template
        body: Option<&'static str>,
        /// `(session_store_key, response_json_field)` extraction pairs
        extractors: &'static [(&'static str, &'static str)],
    },

    /// Branch Step: probabilistic control flow.
    ///
    /// Uses the VU's deterministic RNG to pick a target step index.
    /// `choices` is a list of `(probability, target_step_idx)` pairs.
    /// Probabilities must sum to approximately `1.0`.
    Branch {
        /// `(probability, target_step_idx)` pairs
        choices: Vec<(f64, usize)>,
    },

    /// RetryOnError Step: wraps the next request and retries on `5xx` errors.
    RetryOnError {
        /// Maximum number of retry attempts (must be `> 0`)
        max_retries: u32,
    },

    /// Loop Step: counter-based backwards jump (up to `u32::MAX` iterations).
    Loop {
        /// Number of loop iterations to execute (must be `≥ 1`)
        iterations: u32,
        /// Target step index to jump back to
        jump_back_idx: usize,
    },

    /// Finish Step: terminal state — transitions VU to `Finished`.
    Finish,
}

impl fmt::Display for ScenarioStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Think { distribution } => write!(f, "Think({})", distribution),
            Self::Request {
                method, path, body, ..
            } => {
                if let Some(b) = body {
                    let preview: String = b.chars().take(30).collect();
                    write!(f, "Request({} {}, body={}...)", method, path, preview)
                } else {
                    write!(f, "Request({} {})", method, path)
                }
            }
            Self::Branch { choices } => {
                let s = choices
                    .iter()
                    .map(|(p, idx)| format!("{:.2}→{}", p, idx))
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "Branch({})", s)
            }
            Self::RetryOnError { max_retries } => {
                write!(f, "RetryOnError(max_retries={})", max_retries)
            }
            Self::Loop {
                iterations,
                jump_back_idx,
            } => {
                write!(f, "Loop(iterations={}, goto={})", iterations, jump_back_idx)
            }
            Self::Finish => write!(f, "Finish"),
        }
    }
}

impl ScenarioStep {
    /// Validate this step's parameters for logical consistency.
    ///
    /// # Errors
    ///
    /// Returns [`KrakenError::InvalidScenario`] when:
    /// - `Think(Normal)`: `mean < 0` or `std_dev < 0`
    /// - `Think(Uniform)`: `min > max`
    /// - `Branch`: empty choices, probabilities outside `[0,1]`, or sum ≠ 1.0
    /// - `RetryOnError`: `max_retries == 0`
    /// - `Loop`: `iterations == 0`
    pub fn validate(&self) -> KrakenResult<()> {
        match self {
            Self::Think { distribution } => match distribution {
                ThinkTimeDistribution::Normal { mean, std_dev } => {
                    if *mean < 0.0 || *std_dev < 0.0 {
                        return Err(KrakenError::InvalidScenario(format!(
                            "Think(Normal): mean ({mean}) and std_dev ({std_dev}) must be ≥ 0"
                        )));
                    }
                    Ok(())
                }
                ThinkTimeDistribution::Uniform { min, max } => {
                    if min > max {
                        return Err(KrakenError::InvalidScenario(format!(
                            "Think(Uniform): min ({min}) > max ({max})"
                        )));
                    }
                    Ok(())
                }
            },
            Self::Request { .. } => Ok(()),
            Self::Branch { choices } => {
                if choices.is_empty() {
                    return Err(KrakenError::InvalidScenario(
                        "Branch: must have at least one choice".to_string(),
                    ));
                }
                let total: f64 = choices.iter().map(|(p, _)| p).sum();
                if (total - 1.0).abs() > 0.001 {
                    return Err(KrakenError::InvalidScenario(format!(
                        "Branch: probabilities must sum to ~1.0 (got {:.3})",
                        total
                    )));
                }
                for (p, _) in choices {
                    if !(0.0..=1.0).contains(p) {
                        return Err(KrakenError::InvalidScenario(format!(
                            "Branch: each probability must be in [0.0, 1.0] (got {})",
                            p
                        )));
                    }
                }
                Ok(())
            }
            Self::RetryOnError { max_retries } => {
                if *max_retries == 0 {
                    return Err(KrakenError::InvalidScenario(
                        "RetryOnError: max_retries must be > 0".to_string(),
                    ));
                }
                Ok(())
            }
            Self::Loop { iterations, .. } => {
                if *iterations == 0 {
                    return Err(KrakenError::InvalidScenario(
                        "Loop: iterations must be ≥ 1".to_string(),
                    ));
                }
                Ok(())
            }
            Self::Finish => Ok(()),
        }
    }
}

// ─── Scenario ─────────────────────────────────────────────────────────────────

/// Scenario — immutable execution program for Virtual Users
///
/// A Scenario is a stateless, `'static` instruction sequence shared across all VUs.
/// Each VU maintains its own execution state (program counter, loop counter, wait tick).
///
/// # Design
/// - `name`: compile-time constant for tracing and reporting
/// - `steps`: `&'static [ScenarioStep]` — zero allocation per VU
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Kraken",
        link = "LEP-0012-laplace-interfaces-kraken_deterministic_chaos"
    )
)]
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Scenario {
    /// Human-readable scenario name (compile-time constant)
    pub name: &'static str,
    /// Immutable instruction sequence (shared across all VUs)
    pub steps: &'static [ScenarioStep],
}

impl Scenario {
    /// Create and validate a new scenario.
    ///
    /// Each step's parameters are validated; the first invalid step causes
    /// an error that includes the step index for easy debugging.
    ///
    /// # Errors
    ///
    /// Returns [`KrakenError::InvalidScenario`] if any step fails validation.
    pub fn new(name: &'static str, steps: &'static [ScenarioStep]) -> KrakenResult<Self> {
        for (idx, step) in steps.iter().enumerate() {
            step.validate()
                .map_err(|e| KrakenError::InvalidScenario(format!("Step {idx}: {e}")))?;
        }
        Ok(Self { name, steps })
    }

    /// Number of steps in this scenario
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Returns `true` if the scenario has no steps
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Get the step at `index`, returning `None` if out of bounds
    pub fn step_at(&self, index: usize) -> Option<ScenarioStep> {
        self.steps.get(index).cloned()
    }
}

impl fmt::Display for Scenario {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Scenario: {}", self.name)?;
        for (i, step) in self.steps.iter().enumerate() {
            writeln!(f, "  [{i}] {step}")?;
        }
        Ok(())
    }
}

// ─── ChaosEvent ───────────────────────────────────────────────────────────────

/// Chaos event types for deterministic failure injection
///
/// All chaos injection is fully deterministic and driven by:
/// 1. `VirtualClock`'s logical time (`now_ns()`)
/// 2. Explicit `ChaosEvent` time ranges
/// 3. VU ID ranges for partition events
///
/// This ensures 100% reproducibility: same seed + same time flow = same chaos.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Kraken",
        link = "LEP-0012-laplace-interfaces-kraken_deterministic_chaos"
    )
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChaosEvent {
    /// Inject additional latency during a time window.
    ///
    /// When active, adds `extra_latency_ms` to the request's effective latency
    /// (applied after the network profile's base latency).
    LatencySpike {
        /// Start time in milliseconds (inclusive)
        start_ms: u64,
        /// End time in milliseconds (exclusive)
        end_ms: u64,
        /// Additional latency to inject in milliseconds
        extra_latency_ms: u64,
    },

    /// Partition a range of VUs from network communication.
    ///
    /// When active, the specified VU range receives 503 errors.
    /// VUs outside the range continue normally.
    NetworkPartition {
        /// Start time in milliseconds (inclusive)
        start_ms: u64,
        /// End time in milliseconds (exclusive)
        end_ms: u64,
        /// Range of VU IDs to isolate (e.g., `0..3` for VU 0, 1, 2)
        target_vu_range: std::ops::Range<u64>,
    },
}

impl ChaosEvent {
    /// Returns `true` if this event is active at `current_time_ms`
    pub fn is_active_at_ms(&self, current_time_ms: u64) -> bool {
        match self {
            Self::LatencySpike {
                start_ms, end_ms, ..
            }
            | Self::NetworkPartition {
                start_ms, end_ms, ..
            } => current_time_ms >= *start_ms && current_time_ms < *end_ms,
        }
    }

    /// Returns `true` if this is a `NetworkPartition` that covers `vu_id`
    pub fn affects_vu_id(&self, vu_id: u64) -> bool {
        match self {
            Self::NetworkPartition {
                target_vu_range, ..
            } => target_vu_range.contains(&vu_id),
            _ => false,
        }
    }
}

// ─── ChaosSchedule ────────────────────────────────────────────────────────────

/// Chaos schedule — ordered collection of [`ChaosEvent`]s
///
/// Provides methods to query active events for a given (time, vu_id) pair.
#[derive(Debug, Clone, Default)]
pub struct ChaosSchedule {
    /// Scheduled chaos events
    events: Vec<ChaosEvent>,
}

impl ChaosSchedule {
    /// Create a new, empty schedule
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Add a chaos event to the schedule
    pub fn add_event(&mut self, event: ChaosEvent) {
        self.events.push(event);
    }

    /// Create a schedule pre-populated with the given events
    pub fn from_events(events: Vec<ChaosEvent>) -> Self {
        Self { events }
    }

    /// All active events at `current_time_ms` that apply to `vu_id`
    ///
    /// `LatencySpike` events affect all VUs; `NetworkPartition` events are
    /// filtered to the specified VU ID range.
    pub fn get_active_events(&self, current_time_ms: u64, vu_id: u64) -> Vec<&ChaosEvent> {
        self.events
            .iter()
            .filter(|ev| {
                ev.is_active_at_ms(current_time_ms)
                    && match ev {
                        ChaosEvent::LatencySpike { .. } => true,
                        ChaosEvent::NetworkPartition { .. } => ev.affects_vu_id(vu_id),
                    }
            })
            .collect()
    }

    /// Returns `true` if `vu_id` is partitioned at `current_time_ms`
    pub fn is_partitioned(&self, current_time_ms: u64, vu_id: u64) -> bool {
        self.events.iter().any(|ev| {
            matches!(ev, ChaosEvent::NetworkPartition { .. })
                && ev.is_active_at_ms(current_time_ms)
                && ev.affects_vu_id(vu_id)
        })
    }

    /// Total additional latency (ms) from all active `LatencySpike` events
    pub fn total_extra_latency_ms(&self, current_time_ms: u64) -> u64 {
        self.events
            .iter()
            .filter_map(|ev| {
                if let ChaosEvent::LatencySpike {
                    extra_latency_ms, ..
                } = ev
                {
                    ev.is_active_at_ms(current_time_ms)
                        .then_some(*extra_latency_ms)
                } else {
                    None
                }
            })
            .sum()
    }

    /// Number of events in the schedule
    pub fn event_count(&self) -> usize {
        self.events.len()
    }
}

// ─── RampUpProfile ────────────────────────────────────────────────────────────

/// Load increase patterns for virtual user injection
///
/// Implements the TLA+ `LoadProfile` specification:
/// ```tla
/// LoadProfile == [type: {"Linear", "Step", "Instant"}, target_vus: Nat, duration: Nat]
/// TargetVUsAt(t) == ComputeTargetVUs(profile, t)
/// ```
///
/// All calculations are deterministic given the profile parameters and elapsed time.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Kraken",
        link = "LEP-0012-laplace-interfaces-kraken_deterministic_chaos"
    )
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RampUpProfile {
    /// Linear ramp-up: injects VUs at a constant rate from 0 to `target_vus`.
    ///
    /// `VUs(t) = ceil((t / duration) * target_vus)` for `t ∈ [0, duration_secs]`.
    Linear {
        /// Target number of virtual users to reach
        target_vus: usize,
        /// Ramp-up duration in seconds
        duration_secs: u64,
    },

    /// Step (staircase) ramp-up: adds `step_size` VUs every `step_duration_secs`.
    ///
    /// `VUs(t) = start_vus + floor(t / step_duration) * step_size`
    Step {
        /// Initial VU count
        start_vus: usize,
        /// Number of VUs added per step
        step_size: usize,
        /// Seconds between step increases
        step_duration_secs: u64,
    },

    /// Instant injection: all `target_vus` appear at `t = 0`.
    Instant {
        /// Target number of virtual users to inject immediately
        target_vus: usize,
    },
}

impl RampUpProfile {
    /// Target VU count at `elapsed` time since ramp-up start.
    ///
    /// Return value is monotonically non-decreasing and bounded by the profile.
    pub fn target_vus_at(&self, elapsed: std::time::Duration) -> usize {
        match self {
            Self::Linear {
                target_vus,
                duration_secs,
            } => {
                let total = std::time::Duration::from_secs(*duration_secs);
                if elapsed >= total {
                    *target_vus
                } else if elapsed.as_secs_f64() == 0.0 {
                    0
                } else {
                    let progress = elapsed.as_secs_f64() / total.as_secs_f64();
                    (progress * (*target_vus as f64)).ceil() as usize
                }
            }
            Self::Step {
                start_vus,
                step_size,
                step_duration_secs,
            } => {
                let step_dur = std::time::Duration::from_secs(*step_duration_secs);
                let step_index = (elapsed.as_secs_f64() / step_dur.as_secs_f64()).floor() as u64;
                start_vus + (step_index as usize) * step_size
            }
            Self::Instant { target_vus } => *target_vus,
        }
    }

    /// Estimated total duration of this ramp-up profile.
    ///
    /// For `Instant` profiles this is [`Duration::ZERO`].
    /// For `Step` profiles this is a heuristic (10× the step interval).
    pub fn total_duration(&self) -> std::time::Duration {
        match self {
            Self::Linear { duration_secs, .. } => std::time::Duration::from_secs(*duration_secs),
            Self::Step {
                step_duration_secs, ..
            } => std::time::Duration::from_secs(step_duration_secs * 10),
            Self::Instant { .. } => std::time::Duration::ZERO,
        }
    }
}

impl fmt::Display for RampUpProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Linear {
                target_vus,
                duration_secs,
            } => write!(f, "Linear(0→{} VUs over {}s)", target_vus, duration_secs),
            Self::Step {
                start_vus,
                step_size,
                step_duration_secs,
            } => write!(
                f,
                "Step({}+{}VU every {}s)",
                start_vus, step_size, step_duration_secs
            ),
            Self::Instant { target_vus } => write!(f, "Instant({} VUs)", target_vus),
        }
    }
}
