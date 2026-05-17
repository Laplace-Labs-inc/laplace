//! # Domain Model: Core Business Abstractions
//!
//! This module contains the authoritative definitions of business-critical types
//! that determine request execution characteristics, resource allocation, and
//! feature access across the Laplace platform.
//!
//! ## Type Hierarchy
//!
//! All types in this module are **value types** (`Copy`, `Clone`) representing
//! immutable business classifications. They carry no internal state and are
//! safe to pass by value across async boundaries.
//!
//! - `TenantTier`: Subscription level determining resource limits, SLA, and features
//! - `PriorityLevel`: Request scheduling priority in multi-tenant contexts
//! - `SovereignContext`: The canonical context object passed through all kernel operations,
//!   implementing the Deterministic Context principle
//! - `TransportError`, `KnulStream`, `KnulConnection`, `KnulEndpoint`: QUIC transport
//!   layer contracts for the Kernel Networking Utility Link (KNUL)
//!
//! These types form the basis for the Kernel's scheduler, the Axiom's simulation
//! context, the Kraken's load profile modeling, distributed request tracing,
//! and network communication across the entire Laplace stack.

pub mod context;
/// Entropy domain contracts (seed primitives and Entropy trait)
pub mod entropy;
pub mod kernel;
#[cfg(feature = "twin")]
/// Kraken DSL domain contracts (VUState, ScenarioStep, Scenario, ChaosEvent, RampUpProfile, etc.)
pub mod kraken;
/// Memory domain contracts (Address, Value, CoreId, MemoryBackend, etc.)
pub mod memory;
/// Pool domain contracts (StorageStrategy, HealthStatus)
pub mod pool;
/// QUIC transport layer statistics and diagnostics
pub mod quic;
/// Resource domain contracts (types and tracking traits)
pub mod resource;
pub mod runtime;
/// Scheduler domain contracts (ThreadId, TaskId, SchedulerBackend, etc.)
pub mod scheduler;
pub mod scheduling;
pub mod tenant;
/// Time domain contracts (VirtualTimeNs, LamportClock, ClockBackend, etc.)
pub mod time;
/// Tracing domain contracts (LamportTimestamp, SimulationEvent, TracerBackend, etc.)
pub mod tracing;
pub mod transport;
/// TUI capabilities and permission model (Dumb UI & Closed Kernel pattern)
pub mod tui;

pub use context::{SovereignContext, NO_TURBO_SLOT};
pub use entropy::{ContextId, Entropy, GlobalSeedConfig, LocalSeed, SeedAssignment};
pub use pool::{HealthStatus, StorageStrategy};
pub use resource::{
    RequestResult, ResourceError, ResourceGuard, ResourceId, ResourceTracker, ResourceType,
    ResourceUsage, ThreadId, ThreadStatus,
};
pub use runtime::{RuntimeStats, SovereignRuntime};
pub use scheduling::PriorityLevel;
pub use tenant::{ResourceConfig, TenantMetadata, TenantTier};
pub use transport::{
    HttpMethod, KnulConnection, KnulEndpoint, KnulStream, SovereignTransport, TransportError,
    TransportFactory, TransportHandle, TransportPacket, TransportStats, VirtualRequest,
    VirtualResponse, VirtualTransport, VUID,
};

pub use kernel::KernelCapabilities;

pub use quic::QuicServerStats;

pub use tui::{PanelType, Tier, TuiCapabilities};
