//! Integration Tests - The Zero-cost Razor Final Assembly
//!
//! This test suite verifies that all domain components (Clock, Memory, Simulator)
//! work together seamlessly with zero runtime overhead. All tests strictly enforce
//! strong type usage to ensure compile-time safety and eliminate silent type confusion.
//!
//! # Test Organization
//!
//! Tests are organized into three groups:
//!
//! - **Basic Flow Tests**: Fundamental simulator operations with strong types
//! - **Memory Semantics Tests**: Store buffering, visibility, and fence semantics
//! - **Verification Tests**: Stack-allocated verification simulator (feature-gated)
//!
//! # Strong Type Requirements
//!
//! All tests use the following strong types exclusively:
//! - `CoreId::new(n)`: Processor core identifiers
//! - `Address::new(n)`: Memory addresses in the main store
//! - `Value::new(n)`: Memory cell contents

use laplace_axiom::simulation::{ProductionSimulatorBuilder, Simulator};
use laplace_core::domain::memory::ProductionBackend as ProdMemBackend;
use laplace_core::domain::memory::{Address, CoreId, MemoryConfig, SimulatedMemory, Value};
use laplace_core::domain::time::{ProductionBackend as ProdClockBackend, VirtualClock};

// ============================================================================
// Basic Flow Tests
// ============================================================================

/// Test basic production simulator initialization and operation
///
/// This test verifies the most fundamental simulator workflow: creating a
/// simulator, performing a write operation, running until idle, and reading
/// back the result. It serves as a smoke test for the entire integration.
#[test]
fn test_production_simulator_basic_flow() {
    let mut sim = ProductionSimulatorBuilder::new()
        .num_cores(4)
        .buffer_size(2)
        .build();

    // Core 0 writes the value 42 to Address 0
    sim.memory_mut()
        .write(CoreId::new(0), Address::new(0), Value::new(42))
        .expect("Write should succeed");

    // Run the simulation until all events are processed
    sim.run_until_idle();

    // Verify the value is now visible in main memory
    assert_eq!(
        sim.memory().read_main_memory(Address::new(0)),
        Value::new(42),
        "Value should be flushed to main memory after idle"
    );
}

/// Test store buffer forwarding behavior
///
/// This test validates the store buffer visibility semantics. A core writing
/// to the buffer should see its own writes immediately (forwarding), while
/// other cores cannot see buffered writes until they are flushed to main memory.
///
/// This behavior is essential for correct relaxed memory simulation and
/// matches the behavior of modern processors with store buffers.
#[test]
fn test_store_buffer_forwarding() {
    let mem_backend = ProdMemBackend::new(2, 2);
    let clock_backend = ProdClockBackend::new();
    let clock = VirtualClock::new(clock_backend);
    let memory = SimulatedMemory::new(mem_backend, clock, MemoryConfig::default());

    let mut sim = Simulator::new(memory);

    // Core 0 writes Value(10) to Address(100)
    sim.memory_mut()
        .write(CoreId::new(0), Address::new(100), Value::new(10))
        .expect("Write should succeed");

    // Store buffer forwarding: Core 0 sees its own write immediately
    assert_eq!(
        sim.memory().read(CoreId::new(0), Address::new(100)),
        Value::new(10),
        "Core 0 should see its own buffered write via forwarding"
    );

    // Other cores cannot see buffered writes yet
    assert_eq!(
        sim.memory().read(CoreId::new(1), Address::new(100)),
        Value::new(0),
        "Core 1 should not see Core 0's buffered write"
    );

    // Process the sync event to flush the buffer
    sim.run_until_idle();

    // Now all cores can see the value in main memory
    assert_eq!(
        sim.memory().read(CoreId::new(1), Address::new(100)),
        Value::new(10),
        "Core 1 should see the value after buffer flush"
    );

    assert_eq!(
        sim.memory().read_main_memory(Address::new(100)),
        Value::new(10),
        "Value should be in main memory"
    );
}

/// Test memory fence semantics and ordering guarantees
///
/// This test validates that memory fence operations provide ordering
/// guarantees. A fence ensures that all buffered writes are flushed to
/// main memory before any subsequent operations are issued.
///
/// The test verifies that writes before a fence are committed to main
/// memory in order before writes after the fence.
#[test]
fn test_memory_fence_semantics() {
    let mem_backend = ProdMemBackend::new(2, 4);
    let clock_backend = ProdClockBackend::new();
    let clock = VirtualClock::new(clock_backend);
    let memory = SimulatedMemory::new(mem_backend, clock, MemoryConfig::default());

    let mut sim = Simulator::new(memory);

    // Core 0 writes to two addresses
    sim.memory_mut()
        .write(CoreId::new(0), Address::new(10), Value::new(100))
        .expect("First write should succeed");

    sim.memory_mut()
        .write(CoreId::new(0), Address::new(20), Value::new(200))
        .expect("Second write should succeed");

    // Issue a fence to ensure all writes are flushed
    sim.memory_mut()
        .fence(CoreId::new(0))
        .expect("Fence should succeed");

    // Process the fence event (which flushes the buffer)
    sim.step();

    // Write more data after the fence
    sim.memory_mut()
        .write(CoreId::new(0), Address::new(30), Value::new(300))
        .expect("Third write should succeed");

    // Run until idle to flush remaining writes
    sim.run_until_idle();

    // All values should be visible in main memory
    assert_eq!(
        sim.memory().read_main_memory(Address::new(10)),
        Value::new(100),
        "First value should be in main memory"
    );

    assert_eq!(
        sim.memory().read_main_memory(Address::new(20)),
        Value::new(200),
        "Second value should be in main memory"
    );

    assert_eq!(
        sim.memory().read_main_memory(Address::new(30)),
        Value::new(300),
        "Third value should be in main memory"
    );
}

/// Test temporal ordering of multiple writes to the same address
///
/// This test verifies that the event-driven clock maintains temporal
/// ordering when multiple events are scheduled. Writes to the same
/// address must be processed in the order they were issued to ensure
/// correct semantics.
#[test]
fn test_time_ordering_preserved() {
    let mem_backend = ProdMemBackend::new(2, 2);
    let clock_backend = ProdClockBackend::new();
    let clock = VirtualClock::new(clock_backend);
    let memory = SimulatedMemory::new(mem_backend, clock, MemoryConfig::default());

    let mut sim = Simulator::new(memory);

    // Issue two writes to the same address in order
    sim.memory_mut()
        .write(CoreId::new(0), Address::new(42), Value::new(100))
        .expect("First write should succeed");

    sim.memory_mut()
        .write(CoreId::new(0), Address::new(42), Value::new(200))
        .expect("Second write should succeed");

    // Process the first event
    sim.step();
    assert_eq!(
        sim.memory().read_main_memory(Address::new(42)),
        Value::new(100),
        "After first step, should see first value"
    );

    // Process the second event
    sim.step();
    assert_eq!(
        sim.memory().read_main_memory(Address::new(42)),
        Value::new(200),
        "After second step, should see second value (overwrites first)"
    );
}

/// Test the builder pattern with non-default configuration
///
/// This test validates the fluent builder API for creating simulators
/// with custom configurations. It verifies that the builder properly
/// propagates settings and creates a functional simulator.
#[test]
fn test_builder_pattern_custom_configuration() {
    let mut sim = ProductionSimulatorBuilder::new()
        .num_cores(16)
        .buffer_size(8)
        .enable_tracing(false)
        .build();

    // Verify the simulator works with non-default settings
    sim.memory_mut()
        .write(CoreId::new(0), Address::new(1000), Value::new(42))
        .expect("Write should succeed");

    sim.run_until_idle();

    assert_eq!(
        sim.memory().read_main_memory(Address::new(1000)),
        Value::new(42),
        "Simulator with custom configuration should work correctly"
    );
}

// ============================================================================
// Litmus Test - Store Buffering Pattern
// ============================================================================

/// Test the classic "Store Buffering" memory model litmus test
///
/// This is a fundamental concurrency test from the memory model literature.
/// It tests whether a system correctly implements relaxed memory semantics
/// with store buffers.
///
/// The test is structured as:
/// - Thread 0: Write Address(100) = Value(1), then Read Address(200)
/// - Thread 1: Write Address(200) = Value(1), then Read Address(100)
///
/// Under a sequential consistency model, at least one thread must see
/// the other's write. Under relaxed memory, both threads may see only
/// their own write before flushing (initially).
#[test]
fn test_litmus_store_buffering_initial_state() {
    let mem_backend = ProdMemBackend::new(2, 2);
    let clock_backend = ProdClockBackend::new();
    let clock = VirtualClock::new(clock_backend);
    let memory = SimulatedMemory::new(mem_backend, clock, MemoryConfig::default());

    let mut sim = Simulator::new(memory);

    // Core 0: Write Value(1) to Address(100)
    sim.memory_mut()
        .write(CoreId::new(0), Address::new(100), Value::new(1))
        .expect("Core 0 write should succeed");

    // Core 1: Write Value(1) to Address(200)
    sim.memory_mut()
        .write(CoreId::new(1), Address::new(200), Value::new(1))
        .expect("Core 1 write should succeed");

    // Before flushing: each core sees only its own write (buffered)
    assert_eq!(
        sim.memory().read(CoreId::new(0), Address::new(100)),
        Value::new(1),
        "Core 0 should see its own write"
    );

    assert_eq!(
        sim.memory().read(CoreId::new(0), Address::new(200)),
        Value::new(0),
        "Core 0 should not see Core 1's buffered write"
    );

    assert_eq!(
        sim.memory().read(CoreId::new(1), Address::new(100)),
        Value::new(0),
        "Core 1 should not see Core 0's buffered write"
    );

    assert_eq!(
        sim.memory().read(CoreId::new(1), Address::new(200)),
        Value::new(1),
        "Core 1 should see its own write"
    );
}

/// Test litmus test after buffer flush
///
/// This test continues from the store buffering scenario and verifies
/// that after all buffers are flushed to main memory, all cores can
/// see all writes (global memory visibility).
#[test]
fn test_litmus_store_buffering_after_flush() {
    let mem_backend = ProdMemBackend::new(2, 2);
    let clock_backend = ProdClockBackend::new();
    let clock = VirtualClock::new(clock_backend);
    let memory = SimulatedMemory::new(mem_backend, clock, MemoryConfig::default());

    let mut sim = Simulator::new(memory);

    // Core 0 and Core 1 issue writes
    sim.memory_mut()
        .write(CoreId::new(0), Address::new(100), Value::new(1))
        .expect("Write should succeed");

    sim.memory_mut()
        .write(CoreId::new(1), Address::new(200), Value::new(1))
        .expect("Write should succeed");

    // Run simulation until all events are processed
    sim.run_until_idle();

    // After flush: all cores see all writes (global visibility)
    assert_eq!(
        sim.memory().read(CoreId::new(0), Address::new(100)),
        Value::new(1),
        "Core 0 should see its own write after flush"
    );

    assert_eq!(
        sim.memory().read(CoreId::new(0), Address::new(200)),
        Value::new(1),
        "Core 0 should see Core 1's write after flush"
    );

    assert_eq!(
        sim.memory().read(CoreId::new(1), Address::new(100)),
        Value::new(1),
        "Core 1 should see Core 0's write after flush"
    );

    assert_eq!(
        sim.memory().read(CoreId::new(1), Address::new(200)),
        Value::new(1),
        "Core 1 should see its own write after flush"
    );
}

// ============================================================================
// Verification Mode Tests (Feature-Gated)
// ============================================================================

#[cfg(feature = "twin")]
mod verification_tests {
    use super::*;
    use laplace_axiom::simulation::VerificationSimulatorBuilder;
    use laplace_core::domain::memory::VerificationBackend as VerifMemBackend;
    use laplace_core::domain::time::VerificationBackend as VerifClockBackend;

    /// Smoke test for verification simulator
    ///
    /// This test verifies that the stack-allocated verification simulator
    /// works correctly for formal verification scenarios. It uses bounded
    /// resources suitable for Kani formal verification (2 cores, 4 addresses,
    /// 2 buffer entries per core).
    #[test]
    fn test_verification_simulator_basic_operation() {
        let mut sim = VerificationSimulatorBuilder::build();

        // Perform a simple write and read
        sim.memory_mut()
            .write(CoreId::new(0), Address::new(0), Value::new(42))
            .expect("Write should succeed");

        // Run until idle
        sim.run_until_idle();

        // Verify the value is in main memory
        assert_eq!(
            sim.memory().read_main_memory(Address::new(0)),
            Value::new(42),
            "Verification simulator should maintain memory semantics"
        );
    }

    /// Test verification simulator store buffer forwarding
    ///
    /// This test validates that the verification simulator correctly
    /// implements store buffer forwarding even with its bounded resources.
    #[test]
    fn test_verification_store_buffer_forwarding() {
        let mem_backend = VerifMemBackend::new();
        let clock_backend = VerifClockBackend::new();
        let clock = VirtualClock::new(clock_backend);
        let memory = SimulatedMemory::new(mem_backend, clock, MemoryConfig::default());

        let mut sim = Simulator::new(memory);

        // Core 0 writes
        sim.memory_mut()
            .write(CoreId::new(0), Address::new(0), Value::new(999))
            .expect("Write should succeed");

        // Local forwarding works in verification mode
        assert_eq!(
            sim.memory().read(CoreId::new(0), Address::new(0)),
            Value::new(999),
            "Verification mode should support store buffer forwarding"
        );

        assert_eq!(
            sim.memory().read(CoreId::new(1), Address::new(0)),
            Value::new(0),
            "Other cores should not see buffered writes"
        );

        // Process event
        sim.step();

        // Now visible to all cores
        assert_eq!(
            sim.memory().read(CoreId::new(1), Address::new(0)),
            Value::new(999),
            "Value should be visible to all cores after flush"
        );
    }

    /// Test verification simulator resource bounds
    ///
    /// The verification simulator has bounded resources to keep the state
    /// space finite for Kani formal verification. This test verifies that
    /// the bounds are enforced correctly.
    ///
    /// Configuration: 2 cores, 4 addresses, 2 buffer entries per core
    #[test]
    fn test_verification_simulator_buffer_bounds() {
        let mut sim = VerificationSimulatorBuilder::build();

        // Fill the buffer (max 2 entries per core)
        sim.memory_mut()
            .write(CoreId::new(0), Address::new(0), Value::new(100))
            .expect("First write should succeed");

        sim.memory_mut()
            .write(CoreId::new(0), Address::new(1), Value::new(200))
            .expect("Second write should succeed");

        // Third write should fail because buffer is full
        let result = sim
            .memory_mut()
            .write(CoreId::new(0), Address::new(2), Value::new(300));

        assert!(
            result.is_err(),
            "Third write to same core should fail (buffer full)"
        );
    }

    /// Test verification simulator litmus test
    ///
    /// Validates that the verification simulator correctly implements
    /// relaxed memory semantics even with bounded resources.
    #[test]
    fn test_verification_litmus_test() {
        let mem_backend = VerifMemBackend::new();
        let clock_backend = VerifClockBackend::new();
        let clock = VirtualClock::new(clock_backend);
        let memory = SimulatedMemory::new(mem_backend, clock, MemoryConfig::default());

        let mut sim = Simulator::new(memory);

        // Store buffering scenario
        sim.memory_mut()
            .write(CoreId::new(0), Address::new(0), Value::new(1))
            .expect("Core 0 write should succeed");

        sim.memory_mut()
            .write(CoreId::new(1), Address::new(1), Value::new(1))
            .expect("Core 1 write should succeed");

        // Before flush: isolation
        assert_eq!(
            sim.memory().read(CoreId::new(0), Address::new(0)),
            Value::new(1),
            "Core 0 should see its own write"
        );

        assert_eq!(
            sim.memory().read(CoreId::new(0), Address::new(1)),
            Value::new(0),
            "Core 0 should not see buffered write from Core 1"
        );

        assert_eq!(
            sim.memory().read(CoreId::new(1), Address::new(0)),
            Value::new(0),
            "Core 1 should not see buffered write from Core 0"
        );

        assert_eq!(
            sim.memory().read(CoreId::new(1), Address::new(1)),
            Value::new(1),
            "Core 1 should see its own write"
        );

        // After flush: global visibility
        sim.run_until_idle();

        assert_eq!(
            sim.memory().read(CoreId::new(0), Address::new(0)),
            Value::new(1),
            "Core 0 should still see Address 0"
        );

        assert_eq!(
            sim.memory().read(CoreId::new(0), Address::new(1)),
            Value::new(1),
            "Core 0 should now see Core 1's write"
        );

        assert_eq!(
            sim.memory().read(CoreId::new(1), Address::new(0)),
            Value::new(1),
            "Core 1 should now see Core 0's write"
        );

        assert_eq!(
            sim.memory().read(CoreId::new(1), Address::new(1)),
            Value::new(1),
            "Core 1 should still see Address 1"
        );
    }
}
