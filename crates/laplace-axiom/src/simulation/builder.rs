//! Simulation Builders — Fluent and Typestate Factory Patterns
//!
//! This module provides two complementary builder APIs:
//!
//! - [`ProductionSimulatorBuilder`]: Original fluent builder (backward-compatible).
//! - [`TwinSimulatorBuilder<S>`]: New typestate builder for the Axiom engine.
//!
//! # Choosing a Builder
//!
//! Use [`TwinSimulatorBuilder`] for new code — it enforces correct configuration
//! order at compile time and supports observer registration. Use
//! [`ProductionSimulatorBuilder`] only when interfacing with existing code.

use std::marker::PhantomData;

use laplace_core::domain::time::TimeMode;
use laplace_core::domain::tracing::DEFAULT_MAX_EVENTS;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

#[cfg(any(feature = "verification", feature = "twin"))]
use laplace_core::domain::memory::SimulatedMemory;

#[cfg(any(feature = "verification", feature = "twin"))]
use laplace_core::domain::tracing::{TraceEngine, TraceEngineConfig};

#[cfg(feature = "twin")]
use laplace_core::domain::tracing::VerificationTracer;

use super::hooks::{SimulationObserver, VirtualEnvPlugin};

#[cfg(feature = "verification")]
use super::facade::{ProductionSimulator, TwinSimulator};

#[cfg(any(feature = "verification", feature = "twin"))]
use super::facade::Simulator;

#[cfg(feature = "twin")]
use super::facade::VerificationSimulator;

// ============================================================================
// ProductionSimulatorBuilder (backward-compatible)
// ============================================================================

/// Builder for production simulators — retained for backward compatibility.
///
/// For new code prefer [`TwinSimulatorBuilder`].
///
/// # Example
///
/// ```rust,ignore
/// use laplace_core::domain::simulation::ProductionSimulatorBuilder;
///
/// let mut sim = ProductionSimulatorBuilder::new()
///     .num_cores(8)
///     .buffer_size(4)
///     .build();
/// ```
pub struct ProductionSimulatorBuilder {
    pub(super) num_cores: usize,
    pub(super) buffer_size: usize,
    pub(super) enable_tracing: bool,
    pub(super) max_traced_events: usize,
    pub(super) validate_causality: bool,
}

impl ProductionSimulatorBuilder {
    /// Create builder with sensible defaults (4 cores, buf=2, tracing off).
    pub fn new() -> Self {
        Self {
            num_cores: 4,
            buffer_size: 2,
            enable_tracing: false,
            max_traced_events: DEFAULT_MAX_EVENTS,
            validate_causality: cfg!(debug_assertions),
        }
    }

    /// Set the number of simulated processor cores.
    pub fn num_cores(mut self, num_cores: usize) -> Self {
        self.num_cores = num_cores;
        self
    }

    /// Set the store-buffer capacity per core.
    pub fn buffer_size(mut self, buffer_size: usize) -> Self {
        self.buffer_size = buffer_size;
        self
    }

    /// Enable or disable event tracing.
    pub fn enable_tracing(mut self, enable: bool) -> Self {
        self.enable_tracing = enable;
        self
    }

    /// Set the maximum number of events to retain in the trace.
    pub fn max_traced_events(mut self, max: usize) -> Self {
        self.max_traced_events = max;
        self
    }

    /// Enable runtime happens-before causality validation.
    pub fn validate_causality(mut self, validate: bool) -> Self {
        self.validate_causality = validate;
        self
    }

    /// Build a production simulator.
    #[cfg(feature = "verification")]
    pub fn build(self) -> ProductionSimulator {
        let mem_backend =
            laplace_core::domain::memory::ProductionBackend::new(self.num_cores, self.buffer_size);
        let clock_backend = laplace_core::domain::time::ProductionBackend::new();
        let clock = laplace_core::domain::time::VirtualClock::new(clock_backend);

        let config = laplace_core::domain::memory::MemoryConfig {
            num_cores: self.num_cores,
            max_buffer_size: self.buffer_size,
            consistency_model: laplace_core::domain::memory::ConsistencyModel::Relaxed,
            initial_size: 1024,
        };

        let memory = SimulatedMemory::new(mem_backend, clock, config);
        Simulator::new(memory)
    }

    /// Build a production simulator with an attached tracer.
    #[cfg(feature = "verification")]
    pub fn build_with_tracer(
        self,
    ) -> (
        ProductionSimulator,
        laplace_core::domain::tracing::ProductionTracer,
    ) {
        let max_events = if self.enable_tracing {
            self.max_traced_events
        } else {
            0
        };

        let backend = laplace_core::domain::tracing::ProductionBackend::new(max_events);
        let config = TraceEngineConfig {
            validate_causality: self.validate_causality,
        };

        let tracer = TraceEngine::new(backend, config);
        let simulator = self.build();

        (simulator, tracer)
    }
}

impl Default for ProductionSimulatorBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// VerificationSimulatorBuilder (backward-compatible)
// ============================================================================

/// Builder for Kani verification simulators — fixed configuration for bounded BMC.
#[cfg(feature = "twin")]
pub struct VerificationSimulatorBuilder;

#[cfg(feature = "twin")]
impl VerificationSimulatorBuilder {
    /// Build a verification simulator with the fixed Kani-safe configuration.
    pub fn build() -> VerificationSimulator {
        let mem_backend = laplace_core::domain::memory::VerificationBackend::new();
        let clock_backend = laplace_core::domain::time::VerificationBackend::new();
        let clock = laplace_core::domain::time::VirtualClock::new(clock_backend);

        let config = laplace_core::domain::memory::MemoryConfig {
            num_cores: 2,
            max_buffer_size: 2,
            consistency_model: laplace_core::domain::memory::ConsistencyModel::Relaxed,
            initial_size: 4,
        };

        let memory = SimulatedMemory::new(mem_backend, clock, config);
        Simulator::new(memory)
    }

    /// Build a verification simulator with an attached tracer.
    pub fn build_with_tracer() -> (VerificationSimulator, VerificationTracer) {
        use laplace_core::domain::tracing::VerificationBackend;

        let backend = VerificationBackend::new();
        let config = TraceEngineConfig {
            validate_causality: true,
        };

        let tracer = TraceEngine::new(backend, config);
        let simulator = Self::build();

        (simulator, tracer)
    }
}

// ============================================================================
// TwinSimulatorBuilder — Typestate Pattern
// ============================================================================
//
// State machine:
//   Unconfigured ──cores()──> MemoryReady
//   MemoryReady ──scheduler_threads()──> SchedulerReady
//   SchedulerReady ──finalize()──> FullyConfigured
//   FullyConfigured ──observe()──> FullyConfigured  (additive)
//   FullyConfigured ──build()──> TwinSimulator

/// Typestate marker: builder has not yet received any configuration.
pub struct Unconfigured;

/// Typestate marker: memory parameters (cores, buffer size) have been set.
pub struct MemoryReady;

/// Typestate marker: scheduler thread count has been set.
pub struct SchedulerReady;

/// Typestate marker: all required parameters are set; ready to build.
pub struct FullyConfigured;

/// Internal configuration bag shared across all typestate transitions.
struct BuilderConfig {
    num_cores: usize,
    buffer_size: usize,
    num_scheduler_threads: usize,
    time_mode: TimeMode,
    enable_tracing: bool,
    /// Only consumed by build_with_tracer — suppress dead_code when features are off.
    #[allow(dead_code)]
    max_traced_events: usize,
    validate_causality: bool,
    observers: Vec<Box<dyn SimulationObserver>>,
    plugins: Vec<Box<dyn VirtualEnvPlugin>>,
}

impl BuilderConfig {
    fn defaults() -> Self {
        Self {
            num_cores: 4,
            buffer_size: 2,
            num_scheduler_threads: 4,
            time_mode: TimeMode::EventDriven,
            enable_tracing: false,
            max_traced_events: DEFAULT_MAX_EVENTS,
            validate_causality: cfg!(debug_assertions),
            observers: Vec::new(),
            plugins: Vec::new(),
        }
    }
}

/// Typestate builder for the Axiom engine simulator.
///
/// Enforces correct configuration order at compile time:
///
/// ```text
/// Unconfigured ──cores()──> MemoryReady ──scheduler_threads()──>
///   SchedulerReady ──finalize()──> FullyConfigured ──build()──> TwinSimulator
/// ```
///
/// # Example
///
/// ```rust,ignore
/// use laplace_core::domain::simulation::TwinSimulatorBuilder;
///
/// let mut sim = TwinSimulatorBuilder::new()
///     .cores(4)
///     .scheduler_threads(4)
///     .finalize()
///     .build();
/// ```
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "30_Axiom_Simulation",
        link = "LEP-0010-laplace-axiom-digital_twin_and_typestate"
    )
)]
pub struct TwinSimulatorBuilder<S> {
    config: BuilderConfig,
    _state: PhantomData<S>,
}

// ── Unconfigured ─────────────────────────────────────────────────────────────

impl TwinSimulatorBuilder<Unconfigured> {
    /// Create a new builder with sensible defaults.
    ///
    /// Call [`cores`](Self::cores) to advance to [`MemoryReady`] state.
    pub fn new() -> Self {
        Self {
            config: BuilderConfig::defaults(),
            _state: PhantomData,
        }
    }

    /// Set the number of simulated processor cores and advance to [`MemoryReady`].
    ///
    /// Also accepts a `buffer_size` override via [`MemoryReady::buffer_size`].
    pub fn cores(mut self, num_cores: usize) -> TwinSimulatorBuilder<MemoryReady> {
        self.config.num_cores = num_cores;
        TwinSimulatorBuilder {
            config: self.config,
            _state: PhantomData,
        }
    }
}

impl Default for TwinSimulatorBuilder<Unconfigured> {
    fn default() -> Self {
        Self::new()
    }
}

// ── MemoryReady ───────────────────────────────────────────────────────────────

impl TwinSimulatorBuilder<MemoryReady> {
    /// Override the store-buffer capacity per core (default: 2).
    pub fn buffer_size(mut self, size: usize) -> Self {
        self.config.buffer_size = size;
        self
    }

    /// Set the number of scheduler threads and advance to [`SchedulerReady`].
    pub fn scheduler_threads(mut self, n: usize) -> TwinSimulatorBuilder<SchedulerReady> {
        self.config.num_scheduler_threads = n;
        TwinSimulatorBuilder {
            config: self.config,
            _state: PhantomData,
        }
    }
}

// ── SchedulerReady ────────────────────────────────────────────────────────────

impl TwinSimulatorBuilder<SchedulerReady> {
    /// Enable or disable event tracing.
    pub fn enable_tracing(mut self, enable: bool) -> Self {
        self.config.enable_tracing = enable;
        self
    }

    /// Override the virtual time mode (default: [`TimeMode::EventDriven`]).
    pub fn time_mode(mut self, mode: TimeMode) -> Self {
        self.config.time_mode = mode;
        self
    }

    /// Finalize required configuration and advance to [`FullyConfigured`].
    ///
    /// After this call, observers can be registered and the simulator can be built.
    pub fn finalize(self) -> TwinSimulatorBuilder<FullyConfigured> {
        TwinSimulatorBuilder {
            config: self.config,
            _state: PhantomData,
        }
    }
}

// ── FullyConfigured ───────────────────────────────────────────────────────────

impl TwinSimulatorBuilder<FullyConfigured> {
    /// Register an observer to receive simulation lifecycle events.
    ///
    /// Multiple observers can be registered; they are called in registration order.
    /// This is the only state where `observe()` is available, enforcing that
    /// observers are added after configuration is complete.
    pub fn observe(mut self, observer: impl SimulationObserver + 'static) -> Self {
        self.config.observers.push(Box::new(observer));
        self
    }

    /// Enable or disable event tracing.
    pub fn enable_tracing(mut self, enable: bool) -> Self {
        self.config.enable_tracing = enable;
        self
    }

    /// Enable runtime happens-before causality validation.
    pub fn validate_causality(mut self, validate: bool) -> Self {
        self.config.validate_causality = validate;
        self
    }

    /// Register a [`VirtualEnvPlugin`] that can inject operations or schedule
    /// overrides into the simulation at each step.
    ///
    /// Multiple plugins can be registered; they are called in registration order
    /// before each simulation step.
    pub fn inject(mut self, plugin: impl VirtualEnvPlugin + 'static) -> Self {
        self.config.plugins.push(Box::new(plugin));
        self
    }

    /// Build a [`TwinSimulator`] from the current configuration.
    #[cfg(feature = "verification")]
    pub fn build(self) -> TwinSimulator {
        let inner = self.build_production_sim();
        TwinSimulator::new(inner, self.config.observers, self.config.plugins)
    }

    /// Build a [`TwinSimulator`] with an attached tracer.
    #[cfg(feature = "verification")]
    pub fn build_with_tracer(
        self,
    ) -> (
        TwinSimulator,
        laplace_core::domain::tracing::ProductionTracer,
    ) {
        let max_events = if self.config.enable_tracing {
            self.config.max_traced_events
        } else {
            0
        };

        let backend = laplace_core::domain::tracing::ProductionBackend::new(max_events);
        let config = TraceEngineConfig {
            validate_causality: self.config.validate_causality,
        };
        let tracer = TraceEngine::new(backend, config);
        let inner = self.build_production_sim();
        let sim = TwinSimulator::new(inner, self.config.observers, self.config.plugins);

        (sim, tracer)
    }

    /// Internal: build the underlying `ProductionSimulator`.
    #[cfg(feature = "verification")]
    fn build_production_sim(&self) -> ProductionSimulator {
        let mem_backend = laplace_core::domain::memory::ProductionBackend::new(
            self.config.num_cores,
            self.config.buffer_size,
        );
        let clock_backend = laplace_core::domain::time::ProductionBackend::new();
        let clock = laplace_core::domain::time::VirtualClock::new(clock_backend);

        let config = laplace_core::domain::memory::MemoryConfig {
            num_cores: self.config.num_cores,
            max_buffer_size: self.config.buffer_size,
            consistency_model: laplace_core::domain::memory::ConsistencyModel::Relaxed,
            initial_size: 1024,
        };

        let memory = SimulatedMemory::new(mem_backend, clock, config);
        Simulator::new(memory)
    }
}
