//! Concurrency Integration Tests - Multi-Core Memory Consistency
//!
//! This test suite validates that the simulator correctly models concurrent memory
//! access patterns under relaxed memory consistency with per-core store buffers.
//! The tests verify critical properties including write serialization, buffer overflow
//! handling, and eventual consistency guarantees.
//!
//! # Memory Consistency Model
//!
//! The simulator implements relaxed memory semantics matching modern processors:
//!
//! - Each core has a write buffer that batches writes before flushing to main memory
//! - Writes from one core are not immediately visible to other cores
//! - Within a core, writes are committed to the buffer in order
//! - When the store buffer is flushed (via fence or event processing), writes become
//!   globally visible to all cores
//! - Multiple writes to the same address result in last-write-wins semantics
//!
//! # Test Organization
//!
//! The tests are organized by the memory property they validate:
//!
//! - **Write Serialization**: When multiple cores write to the same address,
//!   the final value is determined by a well-defined order
//! - **Buffer Capacity**: Each core's store buffer has a finite capacity and
//!   rejects writes when full
//! - **Eventual Consistency**: Values written by one core eventually become
//!   visible to other cores after buffer flushing

use laplace_axiom::simulation::ProductionSimulatorBuilder;
use laplace_core::domain::memory::{Address, CoreId, Value};

// ============================================================================
// Test 1: Write Serialization Under Concurrent Access
// ============================================================================

/// Test that concurrent writes to the same address resolve with last-write-wins semantics
///
/// # Scenario
///
/// Two cores concurrently write to the same memory address. Under relaxed memory
/// semantics with store buffers, the writes occur in the buffer independently.
/// When buffers are flushed to main memory, one write overwrites the other.
/// The final value in main memory should be one of the written values, and the
/// system should reach a stable, consistent state.
///
/// # Execution
///
/// - Core 0 writes Value(100) to Address(100)
/// - Core 1 writes Value(200) to Address(100)
/// - Run simulation until idle (flushes all buffers)
/// - Verify main memory contains either 100 or 200 (last-write-wins)
/// - Verify both cores see the same value when reading after flush
///
/// # Verification Guarantee
///
/// This test demonstrates that the simulator correctly implements write
/// serialization and that buffer flushing maintains memory consistency.
/// The final state is deterministic (though which value "wins" depends on
/// the order events are processed from the clock).
#[test]
fn test_write_serialization() {
    let mut sim = ProductionSimulatorBuilder::new()
        .num_cores(2)
        .buffer_size(4)
        .build();

    // Both cores write to the same address
    sim.memory_mut()
        .write(CoreId::new(0), Address::new(100), Value::new(100))
        .expect("Core 0 write should succeed");

    sim.memory_mut()
        .write(CoreId::new(1), Address::new(100), Value::new(200))
        .expect("Core 1 write should succeed");

    // Before flushing: each core sees its own write, but not the other's
    assert_eq!(
        sim.memory().read(CoreId::new(0), Address::new(100)),
        Value::new(100),
        "Core 0 should see its own buffered write"
    );

    assert_eq!(
        sim.memory().read(CoreId::new(1), Address::new(100)),
        Value::new(200),
        "Core 1 should see its own buffered write"
    );

    // Run simulation until all buffers are flushed to main memory
    sim.run_until_idle();

    // After flushing: verify that main memory contains a deterministic final value
    let final_value = sim.memory().read_main_memory(Address::new(100));

    // The final value must be one of the written values (last-write-wins semantics)
    assert!(
        final_value == Value::new(100) || final_value == Value::new(200),
        "Final value must be one of the written values (100 or 200), got {:?}",
        final_value
    );

    // Verify both cores now see the same value (memory consistency established)
    let core0_final_view = sim.memory().read(CoreId::new(0), Address::new(100));
    let core1_final_view = sim.memory().read(CoreId::new(1), Address::new(100));

    assert_eq!(
        core0_final_view, final_value,
        "Core 0 should see the final value after flush"
    );

    assert_eq!(
        core1_final_view, final_value,
        "Core 1 should see the final value after flush"
    );

    println!(
        "✓ Write serialization verified: final value = {:?}",
        final_value
    );
}

// ============================================================================
// Test 2: Store Buffer Capacity and Overflow Handling
// ============================================================================

/// Test that the simulator correctly enforces store buffer capacity limits
///
/// # Scenario
///
/// A core attempts to issue more writes than its store buffer can hold.
/// The buffer has a fixed capacity (configured via buffer_size in the builder).
/// When the buffer reaches capacity, subsequent writes should fail with an error.
/// After flushing one buffered entry, a new write should succeed.
///
/// # Execution
///
/// - Core 0 fills its store buffer with writes (buffer_size = 4)
/// - Verify that writes 1-4 succeed
/// - Verify that write 5 fails (buffer full)
/// - Process one event (flushes one entry from the buffer)
/// - Verify that a new write now succeeds
/// - Run until idle and verify final state
///
/// # Verification Guarantee
///
/// This test ensures the simulator prevents silent data loss by enforcing
/// buffer capacity limits. Applications must manage buffer resources
/// carefully or risk write rejection under load.
#[test]
fn test_buffer_capacity_stress() {
    let buffer_size = 4;
    let mut sim = ProductionSimulatorBuilder::new()
        .num_cores(1)
        .buffer_size(buffer_size)
        .build();

    // Fill the buffer to capacity
    for i in 0..buffer_size {
        let result = sim.memory_mut().write(
            CoreId::new(0),
            Address::new(1000 + i),
            Value::new(100 + i as u64),
        );

        assert!(
            result.is_ok(),
            "Write {} to buffer should succeed (buffer capacity is {})",
            i + 1,
            buffer_size
        );
    }

    // Attempt to exceed buffer capacity
    let overflow_result = sim.memory_mut().write(
        CoreId::new(0),
        Address::new(1000 + buffer_size),
        Value::new(200),
    );

    assert!(
        overflow_result.is_err(),
        "Write exceeding buffer capacity should fail"
    );

    println!("✓ Buffer overflow correctly rejected");

    // Process one event to flush one entry from the buffer
    let event_processed = sim.step();
    assert!(
        event_processed,
        "Processing event should succeed and flush one buffer entry"
    );

    println!("✓ One buffer entry flushed via event processing");

    // Now a new write should succeed (we freed one buffer slot)
    let recovery_result =
        sim.memory_mut()
            .write(CoreId::new(0), Address::new(2000), Value::new(999));

    assert!(
        recovery_result.is_ok(),
        "Write after flushing one entry should succeed"
    );

    println!("✓ New write succeeded after buffer space freed");

    // Run remaining simulation
    sim.run_until_idle();

    // Verify final state: all values are in main memory
    for i in 0..buffer_size {
        let addr = Address::new(1000 + i);
        let expected = Value::new(100 + i as u64);
        let final_value = sim.memory().read_main_memory(addr);
        assert_eq!(
            final_value, expected,
            "Value at address {:?} should be {:?} after flush",
            addr, expected
        );
    }

    let recovery_addr = Address::new(2000);
    let recovery_value = sim.memory().read_main_memory(recovery_addr);
    assert_eq!(
        recovery_value,
        Value::new(999),
        "Recovery write should be in main memory"
    );

    println!("✓ All buffered writes persisted to main memory");
}

// ============================================================================
// Test 3: Interleaved Access and Eventual Consistency
// ============================================================================

/// Test that concurrent reads and writes establish eventual consistency
///
/// # Scenario
///
/// Two cores perform concurrent write and read operations with relaxed memory
/// consistency. Before flushing, reads see only locally buffered writes from
/// the reading core. After flushing, all cores see all writes (eventual
/// consistency).
///
/// # Execution
///
/// Step 1 (Buffering Phase):
/// - Core 0 writes Value(10) to Address(1000)
/// - Core 1 writes Value(20) to Address(2000)
/// - Core 0 reads Address(2000) - should read Value(0) [not yet flushed from Core 1]
/// - Core 1 reads Address(1000) - should read Value(0) [not yet flushed from Core 0]
/// - Core 0 reads its own write at Address(1000) - should read Value(10) [local forward]
/// - Core 1 reads its own write at Address(2000) - should read Value(20) [local forward]
///
/// Step 2 (After Flushing):
/// - Run simulation until idle (all buffers flushed to main memory)
/// - Core 0 reads Address(2000) - should now read Value(20)
/// - Core 1 reads Address(1000) - should now read Value(10)
///
/// # Verification Guarantee
///
/// This test demonstrates the fundamental property of relaxed memory consistency:
/// writes are not immediately visible across cores, but eventual consistency
/// is guaranteed when buffers are flushed. This matches the behavior of real
/// concurrent systems.
#[test]
fn test_interleaved_consistency() {
    let mut sim = ProductionSimulatorBuilder::new()
        .num_cores(2)
        .buffer_size(4)
        .build();

    // Phase 1: Both cores issue writes
    sim.memory_mut()
        .write(CoreId::new(0), Address::new(1000), Value::new(10))
        .expect("Core 0 write to 1000 should succeed");

    sim.memory_mut()
        .write(CoreId::new(1), Address::new(2000), Value::new(20))
        .expect("Core 1 write to 2000 should succeed");

    // Phase 2: Cross-core reads (before buffer flush)
    // Core 0 tries to read what Core 1 wrote - not yet visible
    let core0_reads_core1 = sim.memory().read(CoreId::new(0), Address::new(2000));
    assert_eq!(
        core0_reads_core1,
        Value::new(0),
        "Core 0 should not see Core 1's buffered write yet"
    );

    // Core 1 tries to read what Core 0 wrote - not yet visible
    let core1_reads_core0 = sim.memory().read(CoreId::new(1), Address::new(1000));
    assert_eq!(
        core1_reads_core0,
        Value::new(0),
        "Core 1 should not see Core 0's buffered write yet"
    );

    // Phase 3: Local reads (each core reads its own write via forwarding)
    let core0_reads_own = sim.memory().read(CoreId::new(0), Address::new(1000));
    assert_eq!(
        core0_reads_own,
        Value::new(10),
        "Core 0 should see its own buffered write via store buffer forwarding"
    );

    let core1_reads_own = sim.memory().read(CoreId::new(1), Address::new(2000));
    assert_eq!(
        core1_reads_own,
        Value::new(20),
        "Core 1 should see its own buffered write via store buffer forwarding"
    );

    println!("✓ Pre-flush: isolation verified (each core sees only its own writes)");

    // Phase 4: Flush all buffers to main memory
    sim.run_until_idle();

    println!("✓ All store buffers flushed to main memory");

    // Phase 5: Verify eventual consistency (cross-core reads now succeed)
    let core0_reads_core1_after = sim.memory().read(CoreId::new(0), Address::new(2000));
    assert_eq!(
        core0_reads_core1_after,
        Value::new(20),
        "Core 0 should now see Core 1's write after buffer flush"
    );

    let core1_reads_core0_after = sim.memory().read(CoreId::new(1), Address::new(1000));
    assert_eq!(
        core1_reads_core0_after,
        Value::new(10),
        "Core 1 should now see Core 0's write after buffer flush"
    );

    // Verify main memory consistency
    let main_addr1000 = sim.memory().read_main_memory(Address::new(1000));
    let main_addr2000 = sim.memory().read_main_memory(Address::new(2000));

    assert_eq!(
        main_addr1000,
        Value::new(10),
        "Main memory should contain Core 0's write"
    );

    assert_eq!(
        main_addr2000,
        Value::new(20),
        "Main memory should contain Core 1's write"
    );

    println!("✓ Post-flush: eventual consistency verified (all cores see all writes)");
}

// ============================================================================
// Test 4: Complex Interleaving with Multiple Buffers
// ============================================================================

/// Test complex concurrent patterns with multiple buffers in flight
///
/// # Scenario
///
/// This test simulates a more realistic concurrent scenario where both cores
/// issue multiple operations with complex interleaving patterns. It verifies
/// that the simulator correctly maintains consistency across multiple buffered
/// writes and the flushing process.
///
/// # Execution
///
/// Core 0:
/// - Write Address(100) = Value(1)
/// - Write Address(101) = Value(2)
/// - Read Address(200) (cross-core read)
/// - Read Address(201) (cross-core read)
///
/// Core 1:
/// - Write Address(200) = Value(10)
/// - Write Address(201) = Value(20)
/// - Read Address(100) (cross-core read)
/// - Read Address(101) (cross-core read)
///
/// Then flush and verify final state.
#[test]
fn test_complex_buffer_interleaving() {
    let mut sim = ProductionSimulatorBuilder::new()
        .num_cores(2)
        .buffer_size(4)
        .build();

    // Core 0: Issue writes
    sim.memory_mut()
        .write(CoreId::new(0), Address::new(100), Value::new(1))
        .expect("Core 0 write 1 should succeed");

    sim.memory_mut()
        .write(CoreId::new(0), Address::new(101), Value::new(2))
        .expect("Core 0 write 2 should succeed");

    // Core 1: Issue writes
    sim.memory_mut()
        .write(CoreId::new(1), Address::new(200), Value::new(10))
        .expect("Core 1 write 1 should succeed");

    sim.memory_mut()
        .write(CoreId::new(1), Address::new(201), Value::new(20))
        .expect("Core 1 write 2 should succeed");

    // Before flush: cross-core reads see zeros (writes not yet flushed)
    assert_eq!(
        sim.memory().read(CoreId::new(0), Address::new(200)),
        Value::new(0),
        "Core 0 should not see Core 1's buffered writes"
    );

    assert_eq!(
        sim.memory().read(CoreId::new(0), Address::new(201)),
        Value::new(0),
        "Core 0 should not see Core 1's buffered writes"
    );

    assert_eq!(
        sim.memory().read(CoreId::new(1), Address::new(100)),
        Value::new(0),
        "Core 1 should not see Core 0's buffered writes"
    );

    assert_eq!(
        sim.memory().read(CoreId::new(1), Address::new(101)),
        Value::new(0),
        "Core 1 should not see Core 0's buffered writes"
    );

    // Flush all buffers
    sim.run_until_idle();

    // After flush: all cores see all writes
    assert_eq!(
        sim.memory().read(CoreId::new(0), Address::new(100)),
        Value::new(1)
    );
    assert_eq!(
        sim.memory().read(CoreId::new(0), Address::new(101)),
        Value::new(2)
    );
    assert_eq!(
        sim.memory().read(CoreId::new(0), Address::new(200)),
        Value::new(10)
    );
    assert_eq!(
        sim.memory().read(CoreId::new(0), Address::new(201)),
        Value::new(20)
    );

    assert_eq!(
        sim.memory().read(CoreId::new(1), Address::new(100)),
        Value::new(1)
    );
    assert_eq!(
        sim.memory().read(CoreId::new(1), Address::new(101)),
        Value::new(2)
    );
    assert_eq!(
        sim.memory().read(CoreId::new(1), Address::new(200)),
        Value::new(10)
    );
    assert_eq!(
        sim.memory().read(CoreId::new(1), Address::new(201)),
        Value::new(20)
    );

    // Verify main memory consistency
    assert_eq!(
        sim.memory().read_main_memory(Address::new(100)),
        Value::new(1)
    );
    assert_eq!(
        sim.memory().read_main_memory(Address::new(101)),
        Value::new(2)
    );
    assert_eq!(
        sim.memory().read_main_memory(Address::new(200)),
        Value::new(10)
    );
    assert_eq!(
        sim.memory().read_main_memory(Address::new(201)),
        Value::new(20)
    );

    println!("✓ Complex interleaving verified: all writes persisted correctly");
}
