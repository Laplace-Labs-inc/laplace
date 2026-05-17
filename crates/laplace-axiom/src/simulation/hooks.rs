//! Simulation Hooks — Open Integration Interfaces
//!
//! This module defines the observer and plugin traits that allow external modules
//! to integrate with the Axiom engine without modifying its internals.
//!
//! # Design Principle
//!
//! The Axiom engine exposes event hooks through these traits. Any external module
//! that wants to observe or influence the simulation registers itself via
//! `TwinSimulatorBuilder::observe()`. The Axiom engine itself has no knowledge
//! of who is listening — it simply fires the hooks at well-defined points.
//!
//! # Traits
//!
//! - [`SimulationObserver`]: Read-only observer that receives simulation events.
//! - [`VirtualEnvPlugin`]: Active plugin that can inject operations into the simulation.
//! - [`NullObserver`]: Default no-op observer used when no external observer is registered.

use laplace_core::domain::memory::{Address, CoreId, MemoryBackend, MemoryOp, Value};
use laplace_core::domain::scheduler::{ThreadId, ThreadState};
use laplace_core::domain::time::VirtualTimeNs;

// ============================================================================
// Output Types
// ============================================================================

/// Outcome of a single simulation step.
///
/// Returned by [`super::facade::TwinSimulator::step()`] and passed to
/// [`SimulationObserver::on_step_end`] so observers can inspect the result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepOutcome {
    /// Whether an event was actually processed in this step.
    pub event_processed: bool,
    /// Virtual clock time at the end of the step (nanoseconds).
    pub sim_time_ns: VirtualTimeNs,
}

/// Aggregate report produced at the end of a full simulation run.
///
/// Returned by [`super::facade::TwinSimulator::run_until_idle()`] and passed
/// to [`SimulationObserver::on_simulation_complete`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimReport {
    /// Total number of `step()` calls that returned `event_processed = true`.
    pub steps_executed: u64,
    /// Total number of events dispatched (same as `steps_executed` in current impl).
    pub events_processed: u64,
}

// ============================================================================
// SimulationObserver
// ============================================================================

/// Read-only observer for Axiom engine simulation events.
///
/// Implement this trait to receive notifications at key simulation lifecycle
/// points. All methods have default no-op implementations, so implementors
/// only override the events they care about.
///
/// # Registration
///
/// Register an observer via
/// [`TwinSimulatorBuilder::observe`](super::builder::TwinSimulatorBuilder::observe):
///
/// ```rust,ignore
/// let sim = TwinSimulatorBuilder::new()
///     .cores(4)
///     .scheduler_threads(4)
///     .finalize()
///     .observe(MyObserver::default())
///     .build();
/// ```
///
/// # Thread Safety
///
/// Observers must be `Send + Sync` because `TwinSimulator` may be used from
/// multiple threads. All method calls occur within the simulation's execution
/// context (no re-entrancy).
pub trait SimulationObserver: Send + Sync {
    /// Called immediately before each simulation step begins.
    ///
    /// - `tick`: Monotonically increasing step counter (starts at 0).
    /// - `clock_ns`: Current virtual clock time at step start.
    fn on_step_begin(&mut self, tick: u64, clock_ns: VirtualTimeNs) {
        let _ = (tick, clock_ns);
    }

    /// Called immediately after each simulation step completes.
    ///
    /// - `tick`: Same counter as the corresponding `on_step_begin` call.
    /// - `outcome`: Whether an event was processed and the resulting clock time.
    fn on_step_end(&mut self, tick: u64, outcome: &StepOutcome) {
        let _ = (tick, outcome);
    }

    /// Called when a memory write-sync event causes a store buffer flush.
    ///
    /// - `core`: The core whose store buffer entry was flushed.
    /// - `addr`: Memory address written to main memory.
    /// - `val`: Value committed to main memory.
    fn on_memory_sync(&mut self, core: CoreId, addr: Address, val: Value) {
        let _ = (core, addr, val);
    }

    /// Called when a scheduler thread transitions between states.
    ///
    /// - `thread`: The thread whose state changed.
    /// - `from`: Previous thread state.
    /// - `to`: New thread state.
    fn on_thread_state_change(&mut self, thread: ThreadId, from: ThreadState, to: ThreadState) {
        let _ = (thread, from, to);
    }

    /// Called once when the simulation reaches idle (no more pending events).
    ///
    /// - `report`: Aggregate statistics for the completed run.
    fn on_simulation_complete(&mut self, report: &SimReport) {
        let _ = report;
    }
}

// ============================================================================
// VirtualEnvPlugin
// ============================================================================

/// Active plugin that can inject operations into the Axiom engine simulation.
///
/// Unlike [`SimulationObserver`] (which is read-only), a `VirtualEnvPlugin`
/// can actively modify the simulation environment at each step. This enables
/// external frameworks (developed in future phases) to inject chaos, load,
/// or specific memory/scheduling patterns without modifying Axiom engine internals.
///
/// # Safety Contract
///
/// Plugins must uphold determinism: given the same initial state and seed,
/// a plugin must produce the same injections. Non-deterministic plugins will
/// break the Axiom engine's reproducibility guarantee.
///
/// # Registration
///
/// Plugins are registered separately from observers. See
/// `TwinSimulatorBuilder` for registration methods.
pub trait VirtualEnvPlugin: Send + Sync {
    /// Optionally inject a memory operation before the next simulation step.
    ///
    /// Called once per step, before `EventDispatcher::process_event`. If
    /// `Some(op)` is returned, the operation is applied to `memory` by the
    /// `TwinSimulator` before the normal step proceeds.
    ///
    /// Return `None` to leave memory unchanged for this step.
    fn inject_memory_op(&mut self, tick: u64, memory: &mut dyn MemoryBackend) -> Option<MemoryOp>;

    /// Optionally override scheduler thread selection for the next step.
    ///
    /// Return `Some(thread_id)` to force the scheduler to treat the given
    /// thread as the next to run. Return `None` for default scheduling.
    ///
    /// The `TwinSimulator` applies the override if the returned `ThreadId`
    /// is valid; otherwise the override is silently ignored.
    fn inject_schedule_override(&mut self, tick: u64) -> Option<ThreadId>;

    /// Human-readable plugin identifier (used in logs and diagnostics).
    fn name(&self) -> &'static str;
}

// ============================================================================
// NullObserver
// ============================================================================

/// Default no-op observer that does nothing.
///
/// Used internally by `TwinSimulator` when no observer has been registered.
/// Also useful as a placeholder in tests.
///
/// # Example
///
/// ```rust,ignore
/// use laplace_core::domain::simulation::NullObserver;
/// let _ = NullObserver; // zero-size, zero-overhead
/// ```
pub struct NullObserver;

impl SimulationObserver for NullObserver {}
