//! Simulation Facade — Core Engine and TwinSimulator Wrapper
//!
//! This module contains:
//!
//! - [`EventDispatcher`]: Stateless event processor (processes one clock event per call).
//! - [`Simulator<MB, CB>`]: Low-level simulation controller — owns memory + clock.
//! - [`TwinSimulator`]: High-level facade with observer broadcasting.
//!
//! # Usage
//!
//! Prefer [`TwinSimulator`] for new code. Use [`Simulator`] directly only when
//! you need fine-grained control over memory/clock backends (e.g. Kani proofs).

use laplace_core::domain::memory::{MemoryBackend, SimulatedMemory};
use laplace_core::domain::time::{ClockBackend, EventPayload, ScheduledEvent};
use laplace_core::domain::tracing::{ProductionTracer, TracingLamportTimestamp, TracingThreadId};

#[cfg(feature = "verification")]
use laplace_core::domain::time::VirtualTimeNs;

#[cfg(feature = "verification")]
use super::hooks::{SimReport, SimulationObserver, StepOutcome, VirtualEnvPlugin};

// ============================================================================
// Type Aliases for Convenience
// ============================================================================

/// Production simulator — heap-allocated, optimized for speed.
#[cfg(feature = "verification")]
pub type ProductionSimulator = Simulator<
    laplace_core::domain::memory::ProductionBackend,
    laplace_core::domain::time::ProductionBackend,
>;

/// Verification simulator — stack-allocated, optimized for Kani BMC.
#[cfg(feature = "twin")]
pub type VerificationSimulator = Simulator<
    laplace_core::domain::memory::VerificationBackend,
    laplace_core::domain::time::VerificationBackend,
>;

// ============================================================================
// EventDispatcher
// ============================================================================

/// Stateless event dispatcher for simulation steps.
///
/// Processes a single event from the clock queue on each call to
/// [`process_event`](EventDispatcher::process_event). The dispatcher does not
/// hold any state — it operates entirely through mutable references to the
/// memory system.
///
/// This type enforces the state-transition rules of the core specification:
/// each call processes at most one pending event in arrival order.
// TLA+ correspondence: Tick == /\ eventQueue /= {}
//                               /\ LET e == CHOOSE e \in eventQueue : (...)
//                                  IN ProcessEvent(e)
pub struct EventDispatcher;

impl EventDispatcher {
    /// Create a new event dispatcher.
    pub fn new() -> Self {
        Self
    }

    /// Process a single event from the clock queue.
    ///
    /// Returns `true` if an event was processed, `false` if the clock is idle.
    pub fn process_event<MB: MemoryBackend, CB: ClockBackend>(
        &self,
        memory: &mut SimulatedMemory<MB, CB>,
    ) -> bool {
        let Some(event) = memory.clock_mut().tick() else {
            return false;
        };
        self.dispatch_event(event, memory);
        true
    }

    /// Process all pending events until the clock is idle.
    ///
    /// Includes a safety limit (`max_events`) to prevent unbounded loops in
    /// Kani verification mode.
    ///
    /// Returns the number of events processed.
    pub fn process_all<MB: MemoryBackend, CB: ClockBackend>(
        &self,
        memory: &mut SimulatedMemory<MB, CB>,
    ) -> usize {
        self.process_all_bounded(memory, 1000)
    }

    /// Process events with an explicit upper bound.
    ///
    /// Useful for configuring the bound at builder level instead of relying on
    /// the hard-coded constant.
    pub fn process_all_bounded<MB: MemoryBackend, CB: ClockBackend>(
        &self,
        memory: &mut SimulatedMemory<MB, CB>,
        max_events: usize,
    ) -> usize {
        let mut count = 0;
        while count < max_events && self.process_event(memory) {
            count += 1;
        }
        count
    }

    fn dispatch_event<MB: MemoryBackend, CB: ClockBackend>(
        &self,
        event: ScheduledEvent,
        memory: &mut SimulatedMemory<MB, CB>,
    ) {
        match event.payload {
            EventPayload::MemoryWriteSync { core, .. } => {
                let _ = memory.flush_one(core);
            }
            EventPayload::MemoryFence { core } => {
                while memory.get_buffer_len(core) > 0 {
                    let _ = memory.flush_one(core);
                }
            }
            EventPayload::Test(_)
            | EventPayload::TaskReady { .. }
            | EventPayload::WatchdogTimeout { .. }
            | EventPayload::Custom(_) => {}
        }
    }
}

impl Default for EventDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Simulator
// ============================================================================

/// Low-level simulation controller.
///
/// Owns the entire memory system (which in turn owns the clock). Direct
/// ownership eliminates indirection overhead and makes lifetime tracking
/// tractable for formal verification.
///
/// ```text
/// Simulator<MB, CB>
///   └─ owns ─> SimulatedMemory<MB, CB>
///       └─ owns ─> VirtualClock<CB>
/// ```
///
/// The ownership hierarchy corresponds to the compositional refinement of
/// the core specification modules.
// TLA+ correspondence: Simulation == INSTANCE VirtualClock WITH
//                                      INSTANCE SimulatedMemory
pub struct Simulator<MB: MemoryBackend, CB: ClockBackend> {
    memory: SimulatedMemory<MB, CB>,
    dispatcher: EventDispatcher,
}

impl<MB: MemoryBackend, CB: ClockBackend> Simulator<MB, CB> {
    /// Create a new simulator owning `memory`.
    pub fn new(memory: SimulatedMemory<MB, CB>) -> Self {
        Self {
            memory,
            dispatcher: EventDispatcher::new(),
        }
    }

    /// Immutable reference to the memory system.
    pub fn memory(&self) -> &SimulatedMemory<MB, CB> {
        &self.memory
    }

    /// Mutable reference to the memory system.
    pub fn memory_mut(&mut self) -> &mut SimulatedMemory<MB, CB> {
        &mut self.memory
    }

    /// Process a single simulation step.
    ///
    /// Returns `true` if a step was taken, `false` if simulation is idle.
    pub fn step(&mut self) -> bool {
        self.dispatcher.process_event(&mut self.memory)
    }

    /// Run until all events are processed.
    ///
    /// Returns the number of events processed.
    pub fn run_until_idle(&mut self) -> usize {
        self.dispatcher.process_all(&mut self.memory)
    }

    /// Returns `true` when the clock queue is empty and all store buffers are flushed.
    pub fn is_idle(&self) -> bool {
        self.memory.clock().is_queue_empty() && self.memory.all_buffers_empty()
    }

    /// Reset to initial state (clears memory and clock).
    pub fn reset(&mut self) {
        self.memory.reset();
    }
}

// ============================================================================
// TwinSimulator
// ============================================================================

/// High-level simulation facade with observer broadcasting.
///
/// Wraps a [`ProductionSimulator`] and maintains a list of
/// [`SimulationObserver`]s that receive event notifications on every
/// `step()` call.
///
/// # Building
///
/// Use [`TwinSimulatorBuilder`](super::builder::TwinSimulatorBuilder) to
/// construct a `TwinSimulator`:
///
/// ```rust,ignore
/// use laplace_core::domain::simulation::TwinSimulatorBuilder;
///
/// let mut sim = TwinSimulatorBuilder::new()
///     .cores(4)
///     .scheduler_threads(4)
///     .finalize()
///     .build();
///
/// sim.run_until_idle();
/// ```
#[cfg(feature = "verification")]
pub struct TwinSimulator {
    inner: ProductionSimulator,
    observers: Vec<Box<dyn SimulationObserver>>,
    plugins: Vec<Box<dyn VirtualEnvPlugin>>,
    tick: u64,
    /// Thread override set by a plugin for the current step.
    ///
    /// Written by [`force_next_thread`](Self::force_next_thread) and consumed by
    /// [`take_forced_thread`](Self::take_forced_thread). Cleared at the start of
    /// every [`step`](Self::step) call so stale values never leak across steps.
    forced_thread: Option<laplace_core::domain::scheduler::ThreadId>,
}

#[cfg(feature = "verification")]
impl TwinSimulator {
    /// Create a new `TwinSimulator` wrapping `inner` with the given observers and plugins.
    ///
    /// Prefer [`TwinSimulatorBuilder`](super::builder::TwinSimulatorBuilder)
    /// over calling this directly.
    pub fn new(
        inner: ProductionSimulator,
        observers: Vec<Box<dyn SimulationObserver>>,
        plugins: Vec<Box<dyn VirtualEnvPlugin>>,
    ) -> Self {
        Self {
            inner,
            observers,
            plugins,
            tick: 0,
            forced_thread: None,
        }
    }

    /// Process a single simulation step and notify all observers.
    ///
    /// Calls `on_step_begin` before processing and `on_step_end` after.
    /// Returns a [`StepOutcome`] describing whether an event was processed.
    pub fn step(&mut self) -> StepOutcome {
        let clock_ns = self.inner.memory().clock().current_time();

        for obs in &mut self.observers {
            obs.on_step_begin(self.tick, clock_ns);
        }

        // Clear the previous step's thread override so stale values never leak.
        self.forced_thread = None;

        // Phase 1: collect schedule overrides (first Some wins; all plugins are called).
        // The returned ThreadId is stored and exposed via `take_forced_thread()` so that
        // DporRunner can read which thread the plugin requested for this step.
        let override_thread = self
            .plugins
            .iter_mut()
            .find_map(|p| p.inject_schedule_override(self.tick));
        if let Some(thread) = override_thread {
            self.forced_thread = Some(thread);
        }

        // Phase 2: apply memory injections from all plugins.
        for plugin in &mut self.plugins {
            let _ = plugin.inject_memory_op(self.tick, self.inner.memory_mut().backend_mut());
        }

        let event_processed = self.inner.step();
        let sim_time_ns = self.inner.memory().clock().current_time();
        let outcome = StepOutcome {
            event_processed,
            sim_time_ns,
        };

        for obs in &mut self.observers {
            obs.on_step_end(self.tick, &outcome);
        }

        if event_processed {
            self.tick += 1;
        }

        outcome
    }

    /// Run until idle, notifying observers on each step and at completion.
    ///
    /// Returns an aggregate [`SimReport`].
    pub fn run_until_idle(&mut self) -> SimReport {
        let mut events_processed: u64 = 0;

        loop {
            let outcome = self.step();
            if !outcome.event_processed {
                break;
            }
            events_processed += 1;
        }

        let report = SimReport {
            steps_executed: events_processed,
            events_processed,
        };

        for obs in &mut self.observers {
            obs.on_simulation_complete(&report);
        }

        report
    }

    /// Immutable reference to the underlying memory system.
    pub fn memory(
        &self,
    ) -> &laplace_core::domain::memory::SimulatedMemory<
        laplace_core::domain::memory::ProductionBackend,
        laplace_core::domain::time::ProductionBackend,
    > {
        self.inner.memory()
    }

    /// Mutable reference to the underlying memory system.
    pub fn memory_mut(
        &mut self,
    ) -> &mut laplace_core::domain::memory::SimulatedMemory<
        laplace_core::domain::memory::ProductionBackend,
        laplace_core::domain::time::ProductionBackend,
    > {
        self.inner.memory_mut()
    }

    /// Read the committed (main-memory) value at `addr`, bypassing any
    /// per-core store buffers.
    ///
    /// Returns `Some(value)` always — unwritten addresses return `Value::new(0)`.
    /// Intended for use by `DporRunner`'s `invariant_checker` callback to
    /// inspect program state after each simulation step.
    ///
    /// # Example
    /// ```rust,ignore
    /// // Check that the ticket counter never underflows.
    /// let v = simulator.read_memory(Address::new(0));
    /// assert!(v.map(|v| v.as_u64() <= 100).unwrap_or(true));
    /// ```
    pub fn read_memory(
        &self,
        addr: laplace_core::domain::memory::Address,
    ) -> Option<laplace_core::domain::memory::Value> {
        Some(self.inner.memory().read_main_memory(addr))
    }

    /// Current virtual clock time in nanoseconds.
    pub fn clock_ns(&self) -> VirtualTimeNs {
        self.inner.memory().clock().current_time()
    }

    /// Returns `true` when the simulation has no more pending events.
    pub fn is_idle(&self) -> bool {
        self.inner.is_idle()
    }

    /// Reset the simulation to its initial state.
    pub fn reset(&mut self) {
        self.inner.reset();
        self.tick = 0;
    }

    /// Current step tick counter (increments only on successful steps).
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Force the next step to use the given thread.
    ///
    /// Called by [`DporRunner`](crate::dpor::runner::DporRunner) after a
    /// plugin returns a schedule override so the runner can record which thread was
    /// requested. The stored value is retrieved with [`take_forced_thread`](Self::take_forced_thread).
    ///
    /// Note: the current `Simulator` event queue is clock-ordered and does not
    /// yet enforce thread-level dispatch. Full enforcement is the responsibility
    /// of the `DporRunner` orchestration layer.
    pub fn force_next_thread(&mut self, thread: laplace_core::domain::scheduler::ThreadId) {
        self.forced_thread = Some(thread);
    }

    /// Consume and return the thread override set for the current step.
    ///
    /// Returns `Some(thread)` if a plugin requested a specific thread via
    /// [`inject_schedule_override`](super::hooks::VirtualEnvPlugin::inject_schedule_override)
    /// during the most recent [`step`](Self::step) call, `None` otherwise.
    ///
    /// After this call the stored override is cleared.
    pub fn take_forced_thread(&mut self) -> Option<laplace_core::domain::scheduler::ThreadId> {
        self.forced_thread.take()
    }
}

// ============================================================================
// TracingAdapter
// ============================================================================

/// Helper trait for integrating tracing with simulators.
///
/// Provides ergonomic methods for recording simulation events into a tracer.
/// Implemented in the infrastructure layer where mutable tracer state is available.
pub trait TracingAdapter {
    /// Record a memory read operation.
    fn trace_memory_read(
        &self,
        tracer: &mut ProductionTracer,
        thread_id: TracingThreadId,
        addr: laplace_core::domain::memory::Address,
        value: laplace_core::domain::memory::Value,
        cache_hit: bool,
    );

    /// Record a memory write operation.
    fn trace_memory_write(
        &self,
        tracer: &mut ProductionTracer,
        thread_id: TracingThreadId,
        addr: laplace_core::domain::memory::Address,
        value: laplace_core::domain::memory::Value,
        buffered: bool,
    );

    /// Record a store buffer flush.
    fn trace_buffer_flush(
        &self,
        tracer: &mut ProductionTracer,
        thread_id: TracingThreadId,
        addr: laplace_core::domain::memory::Address,
        value: laplace_core::domain::memory::Value,
    );

    /// Record a clock tick.
    fn trace_clock_tick(
        &self,
        tracer: &mut ProductionTracer,
        thread_id: TracingThreadId,
        prev_timestamp: TracingLamportTimestamp,
        new_timestamp: TracingLamportTimestamp,
    );
}
