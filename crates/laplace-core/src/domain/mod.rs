//! Domain Layer: Pure Business Logic
//!
//! Encapsulates all domain-specific logic and types that form the core
//! business abstractions of the Laplace platform. The domain layer maintains
//! complete independence from infrastructure concerns and serves as the
//! authoritative source for entity definitions, behavior, and state transitions.
//!
//! # Organization
//!
//! The domain layer is organized into behavioral domains, each addressing
//! a distinct aspect of the platform:
//!
//! - **context**: Execution context propagation and deterministic state management
//! - **entropy**: Unified entropy abstraction for production and deterministic simulation
//! - **journal**: Transaction audit trails and execution lifecycle tracking
//! - **memory**: Memory model abstraction with production and verification backends
//! - **pool**: Resource pooling, allocation, and health monitoring
//! - **resource**: Resource quota enforcement and usage accounting
//! - **scheduler**: Thread-Aware Task Scheduling
//! - **tenant**: Tenant identity, tier classification, and policy enforcement
//! - **time**: Time source abstraction enabling production and virtual time
//! - **clock**: Event-driven virtual clock for deterministic simulation
//! - **tracing**: Zero-allocation observability with causality tracking
//! - **simulation**: Event-driven simulator integrating clock and memory
//! - **dpor**: Dynamic Partial Order Reduction for efficient state space exploration
//!
//! Each domain is independent and composed through clear interfaces, enabling
//! independent evolution while maintaining cohesive business logic.
//!
//! # Architectural Principles
//!
//! The domain layer adheres to three core principles as defined by K-ACA:
//!
//! 1. **Fractal Integrity**: Each module is atomically responsible for a single
//!    business concern, decomposable into smaller units without loss of coherence.
//!
//! 2. **Native-First**: Core logic resides in pure Rust with zero infrastructure
//!    dependencies (no V8, Tokio, Sled, or external runtimes). Extension traits
//!    enable implementation on interface types without violating the orphan rule.
//!
//! 3. **Deterministic Context**: All operations explicitly accept context as
//!    parameters, avoiding implicit state propagation and ensuring reproducibility
//!    for verification and simulation.

pub mod benchmark;
pub mod context;
pub mod entropy;
pub mod journal;
pub mod memory;
pub mod pool;
pub mod resource;
pub mod scheduler;
pub mod telemetry;
pub mod tenant;
pub mod time;
pub mod tracing;
pub mod utils;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Public Re-exports: Commonly Used Types
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// Context domain exports
pub use context::ContextBuilder;

// Entropy domain exports
#[cfg(feature = "twin")]
pub use entropy::DeterministicEntropy;
pub use entropy::{Entropy, SystemEntropy};

// Entropy seed primitives (always available)
pub use entropy::seed::{ContextId, GlobalSeedConfig, LocalSeed, SeedAssignment};

// Deterministic RNG with DPOR snapshot support (twin only)
#[cfg(feature = "twin")]
pub use entropy::rng::{DeterministicRng, RngSnapshot};

// Journal domain exports
pub use journal::{LogStatus, TransactionLog};

// Memory domain exports
pub use memory::{
    Address, ConfigurableBackend, ConsistencyModel, CoreId, MemoryBackend, MemoryConfig, MemoryOp,
    StoreEntry, Value,
};

#[cfg(feature = "verification")]
pub use memory::ProductionBackend;

#[cfg(any(test, feature = "twin"))]
pub use memory::VerificationBackend;

// Pool domain exports
pub use pool::{HealthStatus, PoolHealthCheck, PoolPolicy, PoolSnapshot, StorageStrategy};

// Resource domain exports
pub use resource::{ResourceGuard, ResourceType, ResourceUsage};

// Tenant domain exports
pub use tenant::{
    PathPolicy, ResourceConfig, ResourceConfigExt, ResourcePolicy, TenantMetadata,
    TenantMetadataExt, TenantTier, TenantTierExt, TierRecommendationPolicy,
};

pub use scheduler::{
    SchedulerBackend, SchedulerError, SchedulingStrategy, TaskId, ThreadId, ThreadState,
};

#[cfg(feature = "verification")]
pub use scheduler::ProductionBackend as SchedulerProdBackend;

// Time domain exports
pub use time::{
    Clock, ClockBackend, EventId, EventPayload, LamportClock,
    ProductionBackend as TimeProductionBackend, ScheduledEvent, SystemClock, TimeMode,
    VirtualTimeNs,
};

// VirtualClock only exported when needed for testing/verification
#[cfg(any(test, feature = "twin"))]
pub use time::VirtualClock;

#[cfg(feature = "twin")]
pub use time::VerificationBackend as TimeVerificationBackend;

#[cfg(any(test, feature = "twin"))]
pub use scheduler::VerificationBackend as SchedulerVerificationBackend;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tracing Module Re-exports
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub use tracing::{
    // Causality for tracing
    CausalityGraph,
    EventMetadata,
    FenceType,

    HappensBeforeRelation,

    MemoryOperation,
    // Type aliases
    ProductionTracer as ProductionEventTracer,

    // Core types
    SimulationEvent,
    SyncEvent,
    // Traits and errors
    TracerBackend,
    TracingError,

    TracingLamportTimestamp,
    TracingThreadId,
    DEFAULT_MAX_EVENTS,
    // Constants
    MAX_THREADS,
    TRACING_FORMAT_VERSION,
};

// Verification tracer only available with feature = "twin"
#[cfg(feature = "twin")]
pub use tracing::VerificationTracer as VerificationEventTracer;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Utilities & Global State Re-exports
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub use utils::{
    fill_random_bytes,
    generate_random_uuid,

    get_global_tracer_backend,

    next_random_range,
    // Entropy Utilities
    next_random_u64,
    // Time Utilities
    now_ms,
    now_ns,
    now_us,
    // Global Tracer Setter & Getter
    set_global_tracer,
    DEFAULT_IDLE_TIMEOUT_SECS,
    // Constants
    DEFAULT_POOL_SIZE,
    STANDARD_LATENCY_BASELINE_NS,
    TURBO_LATENCY_TARGET_NS,
};

#[cfg(not(kani))]
pub use utils::{set_global_clock, set_global_entropy};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Telemetry Module Re-exports
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// Lock-free counters — always available
pub use telemetry::{EngineMetrics, GlobalTelemetry, VuMetricEvent};

// Metric streaming pipeline — requires tokio (twin feature)
#[cfg(feature = "twin")]
pub use telemetry::MetricCollector;

// Discrete event ring buffer — requires parking_lot (verification feature)
#[cfg(feature = "verification")]
pub use telemetry::{EventRingBuffer, TelemetryEvent};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Benchmark Module Re-exports
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// Phase 4.2: Resource monitoring and efficiency analysis
pub use benchmark::cpi::calculate_cpi;
pub use benchmark::report::BenchmarkReportBuilder;
pub use benchmark::resource_monitor::DefaultResourceMonitor;
pub use benchmark::{
    CPICalculator, EfficiencyTier, MockResourceMonitor, ResourceMetrics, ResourceMonitor,
};

// Phase 4.3: Stability analysis (requires verification feature)
#[cfg(feature = "verification")]
pub use benchmark::nerve::{NerveReport, OutlierDetector, StabilityAnalyzer, StabilityTier};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_utilities_monotonic() {
        let t1 = now_ms();
        let t2 = now_ms();
        assert!(t2 >= t1);

        let t3 = now_us();
        let t4 = now_us();
        assert!(t4 >= t3);

        let t5 = now_ns();
        let t6 = now_ns();
        assert!(t6 >= t5);
    }

    #[test]
    fn test_time_units_consistent() {
        let ms = now_ms();
        let us = now_us();
        let ns = now_ns();

        // Verify consistency across time units (allowing for execution time)
        assert!(us >= ms * 1_000 - 1_000);
        assert!(ns >= us * 1_000 - 1_000_000);
    }

    #[test]
    fn test_domain_exports() {
        // Verify that commonly used types are accessible at domain level
        let _tier = TenantTier::Free;
        let _status = LogStatus::Success;
        let _strategy = StorageStrategy::Standard;
        let _health = HealthStatus::Healthy;
        let _entropy = SystemEntropy::new();

        // Memory module exports
        let _backend = VerificationBackend::new();
        let _model = ConsistencyModel::Relaxed;
        let _entry = StoreEntry::new(Address::new(0), Value::new(42));
    }

    #[test]
    fn test_entropy_utilities() {
        // Test that entropy utilities work with default SystemEntropy
        let val = next_random_u64();
        assert!(val != 0 || next_random_u64() != 0);

        let mut buf = [0u8; 32];
        fill_random_bytes(&mut buf);
        assert!(buf.iter().any(|&b| b != 0));

        let range_val = next_random_range(100);
        assert!(range_val < 100);
    }

    #[test]
    fn test_generate_random_uuid() {
        let uuid1 = generate_random_uuid();
        let uuid2 = generate_random_uuid();

        assert_eq!(uuid1.len(), 36);
        assert_ne!(uuid1, uuid2);

        for ch in uuid1.chars() {
            assert!(ch.is_ascii_hexdigit() || ch == '-');
        }
    }

    #[test]
    fn test_memory_backend_integration() {
        let mut backend = VerificationBackend::new();

        backend.write_main(Address::new(0), Value::new(100));
        assert_eq!(backend.read_main(Address::new(0)), Value::new(100));

        backend
            .buffer_push(
                CoreId::new(0),
                StoreEntry::new(Address::new(1), Value::new(200)),
            )
            .expect("Push");
        assert_eq!(
            backend.buffer_lookup(CoreId::new(0), Address::new(1)),
            Some(Value::new(200))
        );
    }

    #[cfg(feature = "twin")]
    #[test]
    fn test_deterministic_entropy_injection() {
        let entropy = DeterministicEntropy::new(0xCAFEBABE);

        match set_global_entropy(Box::new(entropy.clone())) {
            Ok(_) => {
                let val1 = next_random_u64();
                entropy.reset();
                let val1_again = next_random_u64();

                assert_eq!(val1, val1_again);
            }
            Err(_) => {
                // Skip if already initialized
            }
        }
    }

    #[cfg(feature = "twin")]
    #[test]
    fn test_verification_backend_integration() {
        use crate::domain::VerificationBackend;

        let mut backend = VerificationBackend::new();
        backend.write_main(Address::new(0), Value::new(100));
        assert_eq!(backend.read_main(Address::new(0)), Value::new(100));

        backend
            .buffer_push(
                CoreId::new(0),
                StoreEntry::new(Address::new(1), Value::new(200)),
            )
            .expect("Push");
        assert_eq!(
            backend.buffer_lookup(CoreId::new(0), Address::new(1)),
            Some(Value::new(200))
        );
    }
}
