//! Quickstart — Axiom Deterministic Verification Environment
//!
//! Demonstrates building a deterministic simulation environment with the
//! `TwinSimulatorBuilder` typestate API and verifying memory consistency
//! via `SimReport` statistics.

use laplace_axiom::simulation::{NullObserver, TwinSimulatorBuilder};
use laplace_core::domain::memory::{Address, CoreId, Value};

fn main() {
    // Build a fully-configured Axiom simulation environment.
    // The typestate builder enforces correct configuration order at compile time,
    // guaranteeing deterministic replay without runtime panics.
    let mut sim = TwinSimulatorBuilder::new()
        .cores(4)
        .scheduler_threads(4)
        .finalize()
        .observe(NullObserver)
        .build();

    // Schedule writes; they are buffered and committed by the simulation engine.
    sim.memory_mut()
        .write(CoreId::new(0), Address::new(0), Value::new(42))
        .expect("write must succeed in a fresh environment");
    sim.memory_mut()
        .write(CoreId::new(0), Address::new(1), Value::new(99))
        .expect("write must succeed in a fresh environment");

    // Run to completion and inspect the report.
    let report = sim.run_until_idle();

    assert_eq!(report.events_processed, 2, "each write produces one event");
    assert!(sim.is_idle(), "no pending events after run_until_idle");

    println!(
        "Simulation complete — {} events processed, {} steps",
        report.events_processed, report.steps_executed
    );
    println!(
        "addr[0] = {}, addr[1] = {}",
        sim.memory().read_main_memory(Address::new(0)).0,
        sim.memory().read_main_memory(Address::new(1)).0,
    );
}
