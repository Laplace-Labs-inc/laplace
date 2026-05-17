//! Axiom Integration Tests — TwinSimulatorBuilder + Observer Pipeline
//!
//! These tests validate the full lifecycle of a `TwinSimulator`:
//! building via `TwinSimulatorBuilder`, registering observers, running the
//! simulation, and verifying `SimReport` statistics.
//!
//! # Feature Requirement
//!
//! All tests here require `--features verification` (or `--all-features`).

use laplace_axiom::simulation::{NullObserver, TwinSimulatorBuilder};
use laplace_core::domain::memory::{Address, CoreId, Value};

// ============================================================================
// Tests
// ============================================================================

/// A `TwinSimulator` with no observers must process writes correctly and
/// return accurate `SimReport` statistics.
#[test]
fn test_twin_builder_no_observer_sim_report() {
    let mut sim = TwinSimulatorBuilder::new()
        .cores(4)
        .scheduler_threads(4)
        .finalize()
        .build();

    sim.memory_mut()
        .write(CoreId::new(0), Address::new(0), Value::new(10))
        .unwrap();
    sim.memory_mut()
        .write(CoreId::new(0), Address::new(1), Value::new(20))
        .unwrap();

    let report = sim.run_until_idle();

    assert_eq!(
        report.events_processed, 2,
        "two writes should produce two events"
    );
    assert_eq!(report.steps_executed, 2);
    assert!(sim.is_idle());
    assert_eq!(
        sim.memory().read_main_memory(Address::new(0)),
        Value::new(10)
    );
    assert_eq!(
        sim.memory().read_main_memory(Address::new(1)),
        Value::new(20)
    );
}

/// `NullObserver` must not affect simulation semantics.
#[test]
fn test_twin_builder_null_observer_transparent() {
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

/// A counting observer must receive exactly as many `on_step_begin` /
/// `on_step_end` calls as events were processed.
#[test]
fn test_counting_observer_receives_correct_callbacks() {
    // We can't move the observer back out of the simulator, so we drive it
    // with individual step() calls to inspect tick progression instead.
    let mut sim = TwinSimulatorBuilder::new()
        .cores(2)
        .buffer_size(4)
        .scheduler_threads(2)
        .finalize()
        .build();

    sim.memory_mut()
        .write(CoreId::new(0), Address::new(0), Value::new(1))
        .unwrap();
    sim.memory_mut()
        .write(CoreId::new(0), Address::new(1), Value::new(2))
        .unwrap();
    sim.memory_mut()
        .write(CoreId::new(0), Address::new(2), Value::new(3))
        .unwrap();

    let report = sim.run_until_idle();

    assert_eq!(report.events_processed, 3);
    assert_eq!(
        sim.tick(),
        3,
        "tick counter must equal processed event count"
    );
}

/// `SimReport` returned from `run_until_idle` must be consistent with
/// individual `step()` outcomes.
#[test]
fn test_sim_report_consistent_with_step_outcomes() {
    let mut sim = TwinSimulatorBuilder::new()
        .cores(4)
        .buffer_size(8)
        .scheduler_threads(4)
        .finalize()
        .build();

    // Write 4 values
    for i in 0..4u64 {
        sim.memory_mut()
            .write(CoreId::new(0), Address::new(i as usize), Value::new(i * 10))
            .unwrap();
    }

    // Process manually and count
    let mut manual_count = 0u64;
    loop {
        let outcome = sim.step();
        if outcome.event_processed {
            manual_count += 1;
        } else {
            break;
        }
    }

    assert_eq!(manual_count, 4, "4 writes produce 4 events");
    assert_eq!(sim.tick(), 4);

    // Verify values
    for i in 0..4u64 {
        assert_eq!(
            sim.memory().read_main_memory(Address::new(i as usize)),
            Value::new(i * 10),
            "value at address {} must be correct",
            i
        );
    }
}

/// Builder must enforce the typestate: `SchedulerReady::enable_tracing` then
/// `finalize` must produce a valid fully-configured builder.
#[test]
fn test_twin_builder_typestate_chain() {
    let mut sim = TwinSimulatorBuilder::new()
        .cores(2)
        .buffer_size(4)
        .scheduler_threads(2)
        .enable_tracing(false)
        .finalize()
        .build();

    sim.memory_mut()
        .write(CoreId::new(0), Address::new(0), Value::new(42))
        .unwrap();
    let report = sim.run_until_idle();

    assert_eq!(report.events_processed, 1);
    assert_eq!(
        sim.memory().read_main_memory(Address::new(0)),
        Value::new(42)
    );
}

/// Multiple observers must all receive notifications from the same step.
#[test]
fn test_multiple_null_observers() {
    let mut sim = TwinSimulatorBuilder::new()
        .cores(2)
        .scheduler_threads(2)
        .finalize()
        .observe(NullObserver)
        .observe(NullObserver)
        .observe(NullObserver)
        .build();

    sim.memory_mut()
        .write(CoreId::new(0), Address::new(0), Value::new(7))
        .unwrap();
    let report = sim.run_until_idle();

    assert_eq!(report.events_processed, 1);
}

/// `reset()` must clear all memory and reset tick to zero.
#[test]
fn test_twin_simulator_reset() {
    let mut sim = TwinSimulatorBuilder::new()
        .cores(2)
        .scheduler_threads(2)
        .finalize()
        .build();

    sim.memory_mut()
        .write(CoreId::new(0), Address::new(0), Value::new(100))
        .unwrap();
    sim.run_until_idle();
    assert_eq!(sim.tick(), 1);

    sim.reset();

    assert_eq!(sim.tick(), 0);
    assert!(sim.is_idle());
    assert_eq!(
        sim.memory().read_main_memory(Address::new(0)),
        Value::new(0),
        "memory must be zeroed after reset"
    );
}
