//! Simulation Integration — The Zero-cost Razor Assembly
//!
//! This module brings together Clock and Memory through event-driven simulation
//! while maintaining the Zero-cost Razor principles:
//!
//! - **No Arc/Box**: Direct ownership of backends
//! - **Static Dispatch**: All polymorphism resolved at compile time
//! - **Stack-only (Kani)**: Verification mode uses zero heap allocation
//!
//! # Module Structure
//!
//! | Sub-module | Contents |
//! |------------|----------|
//! | [`facade`] | `EventDispatcher`, `Simulator`, `TwinSimulator` |
//! | [`builder`] | `ProductionSimulatorBuilder`, `TwinSimulatorBuilder<S>` |
//! | [`hooks`] | `SimulationObserver`, `VirtualEnvPlugin`, `NullObserver` |
//!
//! # TLA+ Correspondence
//!
//! ```tla
//! Next ==
//!     \/ Tick          (process next event)
//!     \/ Write         (schedule write sync)
//!     \/ Fence         (schedule fence)
//!     \/ FlushOne      (commit buffer entry)
//! ```

// ── Sub-modules ──────────────────────────────────────────────────────────────

pub mod builder;
pub mod equivalence;
pub mod facade;
pub mod hooks;

/// Deterministic schedule replay (requires `twin` feature for DPOR types).
#[cfg(feature = "twin")]
pub mod replay_plugin;

// ── Re-exports (backward compat + public API) ────────────────────────────────

// Core types — keep at the module root for existing users
pub use facade::{EventDispatcher, Simulator, TracingAdapter};

// Type aliases
#[cfg(feature = "verification")]
pub use facade::ProductionSimulator;

#[cfg(feature = "twin")]
pub use facade::VerificationSimulator;

// TwinSimulator (new high-level API)
#[cfg(feature = "verification")]
pub use facade::TwinSimulator;

// Builders
pub use builder::{FullyConfigured, MemoryReady, SchedulerReady, Unconfigured};
pub use builder::{ProductionSimulatorBuilder, TwinSimulatorBuilder};

#[cfg(feature = "twin")]
pub use builder::VerificationSimulatorBuilder;

// Hooks & output types
pub use hooks::{NullObserver, SimReport, SimulationObserver, StepOutcome, VirtualEnvPlugin};

// Schedule replay plugin (twin-only — depends on DPOR Schedule type)
#[cfg(feature = "twin")]
pub use replay_plugin::ScheduleReplayPlugin;

// Equivalence types (1:1 TLA+ correspondence)
pub use equivalence::{
    EquivalentTo, LamportTimestamp, StoreBufferEntry, ThreadStateEnum, TwinState, VirtualTime,
};

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use laplace_core::domain::memory::{CoreId, MemoryConfig, ProductionBackend as ProdMemBackend};
    use laplace_core::domain::time::{ProductionBackend as ProdClockBackend, VirtualClock};

    #[test]
    fn test_event_dispatcher_basic() {
        use laplace_core::domain::memory::{Address, SimulatedMemory, Value};

        let mem_backend = ProdMemBackend::new(2, 2);
        let clock_backend = ProdClockBackend::new();
        let clock = VirtualClock::new(clock_backend);
        let config = MemoryConfig::default();
        let mut memory = SimulatedMemory::new(mem_backend, clock, config);

        let dispatcher = EventDispatcher::new();

        memory
            .write(CoreId::new(0), Address::new(0), Value::new(100))
            .unwrap();
        assert!(!memory.clock().is_queue_empty());

        assert!(dispatcher.process_event(&mut memory));
        assert_eq!(memory.read_main_memory(Address::new(0)), Value::new(100));
    }

    #[test]
    fn test_simulator_lifecycle() {
        use laplace_core::domain::memory::{Address, SimulatedMemory, Value};

        let mem_backend = ProdMemBackend::new(2, 2);
        let clock_backend = ProdClockBackend::new();
        let clock = VirtualClock::new(clock_backend);
        let config = MemoryConfig::default();
        let memory = SimulatedMemory::new(mem_backend, clock, config);

        let mut sim = Simulator::new(memory);

        assert!(sim.is_idle());

        sim.memory_mut()
            .write(CoreId::new(0), Address::new(0), Value::new(100))
            .unwrap();
        sim.memory_mut()
            .write(CoreId::new(0), Address::new(1), Value::new(200))
            .unwrap();

        assert!(!sim.is_idle());

        let count = sim.run_until_idle();
        assert_eq!(count, 2);

        assert!(sim.is_idle());
        assert_eq!(
            sim.memory().read_main_memory(Address::new(0)),
            Value::new(100)
        );
        assert_eq!(
            sim.memory().read_main_memory(Address::new(1)),
            Value::new(200)
        );
    }

    #[test]
    fn test_simulator_fence() {
        use laplace_core::domain::memory::{Address, SimulatedMemory, Value};

        let mem_backend = ProdMemBackend::new(2, 2);
        let clock_backend = ProdClockBackend::new();
        let clock = VirtualClock::new(clock_backend);
        let config = MemoryConfig::default();
        let memory = SimulatedMemory::new(mem_backend, clock, config);

        let mut sim = Simulator::new(memory);

        sim.memory_mut()
            .write(CoreId::new(0), Address::new(0), Value::new(100))
            .unwrap();
        sim.memory_mut()
            .write(CoreId::new(0), Address::new(1), Value::new(200))
            .unwrap();
        sim.memory_mut().fence(CoreId::new(0)).unwrap();

        sim.run_until_idle();

        assert_eq!(
            sim.memory().read_main_memory(Address::new(0)),
            Value::new(100)
        );
        assert_eq!(
            sim.memory().read_main_memory(Address::new(1)),
            Value::new(200)
        );
        assert!(sim.memory().all_buffers_empty());
    }

    #[test]
    fn test_simulator_reset() {
        use laplace_core::domain::memory::{Address, SimulatedMemory, Value};

        let mem_backend = ProdMemBackend::new(2, 2);
        let clock_backend = ProdClockBackend::new();
        let clock = VirtualClock::new(clock_backend);
        let config = MemoryConfig::default();
        let memory = SimulatedMemory::new(mem_backend, clock, config);

        let mut sim = Simulator::new(memory);

        sim.memory_mut()
            .write(CoreId::new(0), Address::new(0), Value::new(100))
            .unwrap();
        sim.run_until_idle();

        sim.reset();

        assert_eq!(
            sim.memory().read_main_memory(Address::new(0)),
            Value::new(0)
        );
        assert!(sim.is_idle());
    }

    #[test]
    fn test_production_simulator_builder() {
        use laplace_core::domain::memory::{Address, Value};

        let mut sim = ProductionSimulatorBuilder::new()
            .num_cores(8)
            .buffer_size(4)
            .build();

        sim.memory_mut()
            .write(CoreId::new(0), Address::new(100), Value::new(42))
            .unwrap();
        sim.run_until_idle();
        assert_eq!(
            sim.memory().read_main_memory(Address::new(100)),
            Value::new(42)
        );
    }

    #[test]
    fn test_production_simulator_with_tracer() {
        use laplace_core::domain::memory::{Address, Value};

        let (mut sim, _tracer) = ProductionSimulatorBuilder::new()
            .num_cores(4)
            .buffer_size(2)
            .enable_tracing(true)
            .max_traced_events(1000)
            .build_with_tracer();

        sim.memory_mut()
            .write(CoreId::new(0), Address::new(100), Value::new(42))
            .unwrap();
        sim.run_until_idle();
        assert_eq!(
            sim.memory().read_main_memory(Address::new(100)),
            Value::new(42)
        );
    }

    #[test]
    #[cfg(feature = "twin")]
    fn test_verification_simulator_builder() {
        use laplace_core::domain::memory::{Address, Value};

        let mut sim = VerificationSimulatorBuilder::build();

        sim.memory_mut()
            .write(CoreId::new(0), Address::new(0), Value::new(42))
            .unwrap();
        sim.run_until_idle();
        assert_eq!(
            sim.memory().read_main_memory(Address::new(0)),
            Value::new(42)
        );
    }

    #[test]
    #[cfg(feature = "twin")]
    fn test_verification_simulator_with_tracer() {
        use laplace_core::domain::memory::{Address, Value};

        let (mut sim, mut _tracer) = VerificationSimulatorBuilder::build_with_tracer();

        sim.memory_mut()
            .write(CoreId::new(0), Address::new(0), Value::new(42))
            .unwrap();
        sim.run_until_idle();
        assert_eq!(
            sim.memory().read_main_memory(Address::new(0)),
            Value::new(42)
        );

        assert_eq!(_tracer.event_count(), 0);
        assert_eq!(_tracer.global_timestamp().0, 0);
    }

    #[test]
    fn test_builder_configuration() {
        let builder = ProductionSimulatorBuilder::new()
            .num_cores(8)
            .buffer_size(4)
            .enable_tracing(true)
            .max_traced_events(50000)
            .validate_causality(true);

        assert_eq!(builder.num_cores, 8);
        assert_eq!(builder.buffer_size, 4);
        assert!(builder.enable_tracing);
        assert_eq!(builder.max_traced_events, 50000);
        assert!(builder.validate_causality);
    }

    // ── TwinSimulatorBuilder tests ────────────────────────────────────────────

    #[test]
    fn test_twin_builder_typestate_builds() {
        use laplace_core::domain::memory::{Address, Value};

        let mut sim = TwinSimulatorBuilder::new()
            .cores(4)
            .scheduler_threads(4)
            .finalize()
            .build();

        sim.memory_mut()
            .write(CoreId::new(0), Address::new(0), Value::new(77))
            .unwrap();
        let report = sim.run_until_idle();

        assert_eq!(
            sim.memory().read_main_memory(Address::new(0)),
            Value::new(77)
        );
        assert_eq!(report.events_processed, 1);
    }

    #[test]
    fn test_twin_builder_with_null_observer() {
        use laplace_core::domain::memory::{Address, Value};

        let mut sim = TwinSimulatorBuilder::new()
            .cores(2)
            .scheduler_threads(2)
            .finalize()
            .observe(NullObserver)
            .build();

        sim.memory_mut()
            .write(CoreId::new(0), Address::new(5), Value::new(99))
            .unwrap();
        let report = sim.run_until_idle();
        assert_eq!(report.events_processed, 1);
        assert!(sim.is_idle());
    }

    #[test]
    fn test_twin_simulator_step_and_tick() {
        use laplace_core::domain::memory::{Address, Value};

        let mut sim = TwinSimulatorBuilder::new()
            .cores(2)
            .scheduler_threads(2)
            .finalize()
            .build();

        sim.memory_mut()
            .write(CoreId::new(0), Address::new(0), Value::new(1))
            .unwrap();
        sim.memory_mut()
            .write(CoreId::new(0), Address::new(1), Value::new(2))
            .unwrap();

        let out1 = sim.step();
        assert!(out1.event_processed);
        assert_eq!(sim.tick(), 1);

        let out2 = sim.step();
        assert!(out2.event_processed);
        assert_eq!(sim.tick(), 2);

        let out3 = sim.step();
        assert!(!out3.event_processed);
        assert_eq!(sim.tick(), 2); // tick does not increment on idle step
    }
}
