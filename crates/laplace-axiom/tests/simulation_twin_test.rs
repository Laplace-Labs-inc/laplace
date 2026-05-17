#![cfg(feature = "twin")]
//! Simulation Axiom Integration Tests — Sprint 3 (S-SIM1 ~ S-SIM7)
//!
//! Validates concurrent memory consistency, determinism, fence semantics,
//! and high-concurrency resilience of the `Simulator` under the `twin` feature.
//!
//! # Test Coverage
//!
//! | ID     | Name                           | Verifies |
//! |--------|--------------------------------|----------|
//! | S-SIM1 | `sim_single_vu_determinism`    | Two identical simulators produce identical traces |
//! | S-SIM2 | `sim_run_until_idle_terminates`| `run_until_idle()` always terminates |
//! | S-SIM3 | `sim_multi_core_write_ordering`| Last-write-wins under multi-core contention |
//! | S-SIM4 | `sim_memory_fence_clears_buffer` | `MemoryFence` event fully drains the store buffer |
//! | S-SIM5 | `sim_read_after_write_consistency` | TSO store-buffer forwarding within same core |
//! | S-SIM6 | `sim_reset_clears_all_state`   | `reset()` restores simulator to clean initial state |
//! | S-SIM7 | `sim_high_concurrency_no_panic`| 8-core / 10 000 event stress test never panics |

use laplace_axiom::simulation::ProductionSimulatorBuilder;
use laplace_core::domain::memory::{Address, CoreId, Value};
use laplace_interfaces::domain::kraken::{ChaosEvent, ChaosSchedule};

// ── S-SIM1 ────────────────────────────────────────────────────────────────────

/// Two simulators built with the same configuration and driven with the same
/// write sequence must converge to bitwise-identical memory states.
///
/// # Determinism Guarantee
///
/// `Simulator` is event-driven with a monotonically increasing Lamport clock.
/// Given identical write sequences, the event queue ordering is identical,
/// so both simulators process events in the same order and commit identical
/// values to main memory.
#[test]
fn sim_single_vu_determinism() {
    let mut sim1 = ProductionSimulatorBuilder::new()
        .num_cores(4)
        .buffer_size(4)
        .build();
    let mut sim2 = ProductionSimulatorBuilder::new()
        .num_cores(4)
        .buffer_size(4)
        .build();

    // Deterministic write sequence applied identically to both simulators
    let writes: &[(usize, usize, u64)] = &[
        (0, 100, 10),
        (1, 200, 20),
        (2, 300, 30),
        (3, 400, 40),
        (0, 101, 11),
        (1, 201, 21),
        (2, 301, 31),
        (3, 401, 41),
    ];

    for &(core, addr, val) in writes {
        sim1.memory_mut()
            .write(CoreId::new(core), Address::new(addr), Value::new(val))
            .expect("sim1 write must succeed");
        sim2.memory_mut()
            .write(CoreId::new(core), Address::new(addr), Value::new(val))
            .expect("sim2 write must succeed");
    }

    // Drive both simulators to idle
    let events1 = sim1.run_until_idle();
    let events2 = sim2.run_until_idle();

    // Both must process the same number of events
    assert_eq!(
        events1, events2,
        "Both simulators must process the same number of events"
    );

    // Both must produce identical main-memory states
    for &(_, addr, expected_val) in writes {
        let v1 = sim1.memory().read_main_memory(Address::new(addr));
        let v2 = sim2.memory().read_main_memory(Address::new(addr));
        assert_eq!(
            v1, v2,
            "Address {} must have identical value in both simulators",
            addr
        );
        assert_eq!(
            v1,
            Value::new(expected_val),
            "Address {} must contain expected value {}",
            addr,
            expected_val
        );
    }
}

// ── S-SIM2 ────────────────────────────────────────────────────────────────────

/// After injecting a bounded number of events, `run_until_idle()` must return
/// within finite steps and leave the simulator in the idle state.
///
/// # Termination Guarantee
///
/// The `EventDispatcher::process_all` enforces an internal cap of 1 000 events
/// per call, ensuring no infinite loop. This test verifies that after all events
/// are injected and processed, `is_idle()` returns `true`.
#[test]
fn sim_run_until_idle_terminates() {
    let mut sim = ProductionSimulatorBuilder::new()
        .num_cores(4)
        .buffer_size(4)
        .build();

    // Inject events across all four cores
    for i in 0..10_usize {
        sim.memory_mut()
            .write(
                CoreId::new(i % 4),
                Address::new(i * 10),
                Value::new(i as u64 + 1),
            )
            .expect("Write must succeed");
    }

    assert!(
        !sim.is_idle(),
        "Simulator must have pending events before run_until_idle"
    );

    // run_until_idle must terminate and return > 0 events processed
    let processed = sim.run_until_idle();

    assert!(
        processed > 0,
        "run_until_idle must have processed at least one event"
    );
    assert!(sim.is_idle(), "Simulator must be idle after run_until_idle");
}

// ── S-SIM3 ────────────────────────────────────────────────────────────────────

/// When two cores issue writes to the same address, the final value in main
/// memory must be exactly one of the two written values (last-write-wins).
/// After flushing, all cores must observe the same consistent value.
///
/// # Consistency Guarantee
///
/// The simulator implements relaxed memory (TSO-like store buffers). On flush,
/// writes are committed to main memory in event-queue order. Both cores must
/// see the same final value once all buffers are drained.
#[test]
fn sim_multi_core_write_ordering() {
    let mut sim = ProductionSimulatorBuilder::new()
        .num_cores(2)
        .buffer_size(4)
        .build();

    let shared_addr = Address::new(0);
    let val_core0 = Value::new(100);
    let val_core1 = Value::new(200);

    // Both cores write to the same address (concurrent contention)
    sim.memory_mut()
        .write(CoreId::new(0), shared_addr, val_core0)
        .expect("Core 0 write must succeed");
    sim.memory_mut()
        .write(CoreId::new(1), shared_addr, val_core1)
        .expect("Core 1 write must succeed");

    // Flush all store buffers to main memory
    sim.run_until_idle();

    // Final value must be one of the two written values
    let final_val = sim.memory().read_main_memory(shared_addr);
    assert!(
        final_val == val_core0 || final_val == val_core1,
        "Final value must be one of the two written values (100 or 200), got {:?}",
        final_val
    );

    // Both cores must now agree on the same value (memory consistency established)
    let core0_view = sim.memory().read(CoreId::new(0), shared_addr);
    let core1_view = sim.memory().read(CoreId::new(1), shared_addr);
    assert_eq!(
        core0_view, final_val,
        "Core 0 must see the consistent final value"
    );
    assert_eq!(
        core1_view, final_val,
        "Core 1 must see the consistent final value"
    );
}

// ── S-SIM4 ────────────────────────────────────────────────────────────────────

/// A `MemoryFence` event must drain all pending entries in the issuing core's
/// store buffer. After fence processing, `get_buffer_len` must return zero and
/// the written values must be visible in main memory.
///
/// # Fence Semantics
///
/// `memory_mut().fence(core)` schedules a `MemoryFence` event in the clock.
/// `EventDispatcher` processes it by calling `flush_one` in a loop until the
/// store buffer for that core is empty.
#[test]
fn sim_memory_fence_clears_buffer() {
    let mut sim = ProductionSimulatorBuilder::new()
        .num_cores(2)
        .buffer_size(4)
        .build();

    let core = CoreId::new(0);
    let addr1 = Address::new(10);
    let addr2 = Address::new(11);

    // Write two entries — both remain in the store buffer (not yet flushed)
    sim.memory_mut()
        .write(core, addr1, Value::new(42))
        .expect("First write must succeed");
    sim.memory_mut()
        .write(core, addr2, Value::new(99))
        .expect("Second write must succeed");

    // Before fence: store buffer has pending entries
    let buf_before = sim.memory().get_buffer_len(core);
    assert!(
        buf_before > 0,
        "Store buffer must have pending writes before fence (got {})",
        buf_before
    );

    // Issue a MemoryFence event — schedules complete flush of core's buffer
    sim.memory_mut().fence(core).expect("Fence must succeed");

    // Process all events (MemoryWriteSync × 2 then MemoryFence)
    sim.run_until_idle();

    // After fence: core's store buffer must be completely empty
    assert_eq!(
        sim.memory().get_buffer_len(core),
        0,
        "Store buffer must be empty after MemoryFence processing"
    );

    // Written values must be visible in main memory
    assert_eq!(
        sim.memory().read_main_memory(addr1),
        Value::new(42),
        "First value must be in main memory after fence"
    );
    assert_eq!(
        sim.memory().read_main_memory(addr2),
        Value::new(99),
        "Second value must be in main memory after fence"
    );
}

// ── S-SIM5 ────────────────────────────────────────────────────────────────────

/// Under the TSO (Total Store Order) model, a core must be able to read its
/// own most-recent write from the store buffer before it is flushed to main
/// memory (store-buffer forwarding). Other cores must not see the write until
/// the buffer is flushed.
///
/// # TSO Forwarding Guarantee
///
/// `memory.read(core, addr)` checks the issuing core's store buffer first.
/// If a pending entry for `addr` exists, the buffered value is returned
/// immediately without consulting main memory.
#[test]
fn sim_read_after_write_consistency() {
    let mut sim = ProductionSimulatorBuilder::new()
        .num_cores(2)
        .buffer_size(4)
        .build();

    let core0 = CoreId::new(0);
    let core1 = CoreId::new(1);
    let addr = Address::new(42);

    // Initially both main memory and all buffers are zero
    assert_eq!(
        sim.memory().read_main_memory(addr),
        Value::new(0),
        "Main memory must be 0 before any write"
    );

    // Core 0 writes — value goes into store buffer, not yet in main memory
    sim.memory_mut()
        .write(core0, addr, Value::new(999))
        .expect("Write must succeed");

    // Core 0 reads its own write via store-buffer forwarding
    assert_eq!(
        sim.memory().read(core0, addr),
        Value::new(999),
        "Core 0 must see its own buffered write via store-buffer forwarding"
    );

    // Main memory still has the old value — write not yet flushed
    assert_eq!(
        sim.memory().read_main_memory(addr),
        Value::new(0),
        "Main memory must still have old value (write not yet flushed)"
    );

    // Core 1 cannot see Core 0's buffered write yet
    assert_eq!(
        sim.memory().read(core1, addr),
        Value::new(0),
        "Core 1 must not see Core 0's unflushed write"
    );

    // Flush: now main memory is updated
    sim.run_until_idle();

    assert_eq!(
        sim.memory().read_main_memory(addr),
        Value::new(999),
        "Main memory must contain written value after flush"
    );

    // Both cores now see the flushed value
    assert_eq!(
        sim.memory().read(core0, addr),
        Value::new(999),
        "Core 0 must still see the correct value after flush"
    );
    assert_eq!(
        sim.memory().read(core1, addr),
        Value::new(999),
        "Core 1 must now see Core 0's value after flush"
    );
}

// ── S-SIM6 ────────────────────────────────────────────────────────────────────

/// `reset()` must return the simulator to a clean initial state:
/// - `is_idle()` returns `true`
/// - All store buffers are empty
/// - All main memory locations read as `Value::new(0)`
#[test]
fn sim_reset_clears_all_state() {
    let mut sim = ProductionSimulatorBuilder::new()
        .num_cores(4)
        .buffer_size(4)
        .build();

    // Write distinct values to four separate addresses on four cores
    for i in 0..4_usize {
        sim.memory_mut()
            .write(
                CoreId::new(i),
                Address::new(i * 100),
                Value::new(i as u64 + 1),
            )
            .expect("Write must succeed");
    }

    sim.run_until_idle();

    // Confirm data was committed
    assert_eq!(
        sim.memory().read_main_memory(Address::new(0)),
        Value::new(1),
        "Value must be in main memory before reset"
    );

    // ── Reset ──────────────────────────────────────────────────────────────
    sim.reset();

    // Simulation must be idle (clock queue empty, all buffers drained)
    assert!(sim.is_idle(), "Simulation must be idle after reset");

    // All store buffers must be empty
    assert!(
        sim.memory().all_buffers_empty(),
        "All store buffers must be empty after reset"
    );

    // All written addresses must read as zero (memory cleared)
    for i in 0..4_usize {
        assert_eq!(
            sim.memory().read_main_memory(Address::new(i * 100)),
            Value::new(0),
            "Address {} must be zero after reset",
            i * 100
        );
    }
}

// ── S-SIM7 ────────────────────────────────────────────────────────────────────

/// Under 8-core concurrency with 10 000 injected events driven by a
/// `ChaosSchedule`, the simulator must never panic and must reach idle state.
///
/// # Stress Test Design
///
/// A `ChaosSchedule` with a `LatencySpike` and a `NetworkPartition` event is
/// used to determine which write pattern each virtual core applies at each
/// virtual-time step. 1 250 rounds × 8 cores = 10 000 write events.
///
/// Store buffers are drained every 4 rounds (before overflow at capacity 4)
/// and once more after all rounds complete.
#[test]
fn sim_high_concurrency_no_panic() {
    const NUM_CORES: usize = 8;
    const BUFFER_SIZE: usize = 4;
    const ROUNDS: usize = 1_250; // 1 250 × 8 = 10 000 total writes

    let mut sim = ProductionSimulatorBuilder::new()
        .num_cores(NUM_CORES)
        .buffer_size(BUFFER_SIZE)
        .build();

    // Build ChaosSchedule to model load-test chaos patterns
    let mut chaos = ChaosSchedule::new();
    chaos.add_event(ChaosEvent::LatencySpike {
        start_ms: 0,
        end_ms: 5_000,
        extra_latency_ms: 50,
    });
    chaos.add_event(ChaosEvent::NetworkPartition {
        start_ms: 2_000,
        end_ms: 8_000,
        target_vu_range: 0..4,
    });

    let mut total_writes: usize = 0;

    for round in 0..ROUNDS {
        let time_ms = (round as u64) * 10;

        for core_id in 0..NUM_CORES {
            let active_events = chaos.get_active_events(time_ms, core_id as u64);

            // Chaos-driven address selection: spread writes under normal conditions,
            // concentrate on base address during chaos to create contention
            let addr = if active_events.is_empty() {
                Address::new((core_id * 8 + round % 8) % 128)
            } else {
                Address::new(core_id % 128)
            };

            let val = Value::new((round * NUM_CORES + core_id) as u64 + 1);

            if sim
                .memory_mut()
                .write(CoreId::new(core_id), addr, val)
                .is_err()
            {
                // Buffer full — drain all pending events and retry
                sim.run_until_idle();
                let _ = sim.memory_mut().write(CoreId::new(core_id), addr, val);
            }

            total_writes += 1;
        }

        // Drain store buffers every BUFFER_SIZE rounds to prevent overflow
        if round % BUFFER_SIZE == BUFFER_SIZE - 1 {
            sim.run_until_idle();
        }
    }

    // Final drain: commit all remaining buffered writes
    sim.run_until_idle();

    assert_eq!(
        total_writes,
        ROUNDS * NUM_CORES,
        "Must have attempted exactly 10,000 writes"
    );
    assert!(
        sim.is_idle(),
        "Simulation must be idle after all 10,000 events have been processed"
    );
    assert!(
        sim.memory().all_buffers_empty(),
        "All store buffers must be empty after stress test"
    );
}
