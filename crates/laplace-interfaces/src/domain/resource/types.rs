//! Core types for resource tracking

use std::fmt;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Thread identifier (maps to TLA+ Threads)
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Resource",
        link = "LEP-0004-laplace-interfaces-resource_domain_contracts"
    )
)]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ThreadId(pub usize);

impl ThreadId {
    /// Creates a `ThreadId` from a raw index.
    ///
    /// - **Arguments:** `id` — zero-based thread index within the tracker.
    /// - **Returns:** A new `ThreadId` wrapping `id`.
    /// - **Ownership:** `id` is copied (primitive).
    #[inline(always)]
    pub const fn new(id: usize) -> Self {
        Self(id)
    }

    /// Returns the inner `usize` index.
    ///
    /// - **Returns:** The raw thread index originally passed to [`ThreadId::new`].
    /// - **Ownership:** `self` is copied (cheap `Copy` type).
    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

impl fmt::Display for ThreadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "t{}", self.0)
    }
}

/// Resource identifier (maps to TLA+ Resources)
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Resource",
        link = "LEP-0004-laplace-interfaces-resource_domain_contracts"
    )
)]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ResourceId(pub usize);

impl ResourceId {
    /// Creates a `ResourceId` from a raw index.
    ///
    /// - **Arguments:** `id` — zero-based resource index within the tracker.
    /// - **Returns:** A new `ResourceId` wrapping `id`.
    /// - **Ownership:** `id` is copied (primitive).
    #[inline(always)]
    pub const fn new(id: usize) -> Self {
        Self(id)
    }

    /// Returns the inner `usize` index.
    ///
    /// - **Returns:** The raw resource index originally passed to [`ResourceId::new`].
    /// - **Ownership:** `self` is copied (cheap `Copy` type).
    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

impl fmt::Display for ResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r{}", self.0)
    }
}

/// Thread status (maps to TLA+ ThreadStatusValues)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadStatus {
    /// Thread is actively executing and may request or release resources.
    Running,
    /// Thread is waiting for a resource held by another thread.
    Blocked,
    /// Thread has completed all operations and released its resources.
    Finished,
}

/// Resource types tracked by ResourceGuard
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Resource",
        link = "LEP-0004-laplace-interfaces-resource_domain_contracts"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceType {
    /// CPU execution time, measured in microseconds.
    CpuMicroseconds,
    /// Heap memory allocated, measured in bytes.
    MemoryBytes,
    /// Network bandwidth consumed, measured in bytes.
    NetworkBytes,
    /// Number of simultaneous in-flight requests.
    ConcurrentRequests,
    /// Persistent storage used, measured in bytes.
    StorageBytes,
}

impl ResourceType {
    /// Returns the default resource cap for the Free tier.
    ///
    /// - **Returns:** Per-resource usage limit applicable to free-tier tenants.
    /// - **Ownership:** `self` is immutably borrowed.
    pub fn default_limit_free(&self) -> u64 {
        match self {
            ResourceType::CpuMicroseconds => 100_000,
            ResourceType::MemoryBytes => 32 * 1024 * 1024,
            ResourceType::NetworkBytes => 10 * 1024 * 1024,
            ResourceType::ConcurrentRequests => 5,
            ResourceType::StorageBytes => 100 * 1024 * 1024,
        }
    }

    /// Returns the default resource cap for the Pro tier.
    ///
    /// - **Returns:** Per-resource usage limit applicable to pro-tier tenants.
    /// - **Ownership:** `self` is immutably borrowed.
    pub fn default_limit_pro(&self) -> u64 {
        match self {
            ResourceType::CpuMicroseconds => 1_000_000,
            ResourceType::MemoryBytes => 256 * 1024 * 1024,
            ResourceType::NetworkBytes => 100 * 1024 * 1024,
            ResourceType::ConcurrentRequests => 100,
            ResourceType::StorageBytes => 10 * 1024 * 1024 * 1024,
        }
    }

    /// Returns the default resource cap for the Enterprise tier.
    ///
    /// - **Returns:** Per-resource usage limit applicable to enterprise-tier tenants.
    /// - **Ownership:** `self` is immutably borrowed.
    pub fn default_limit_enterprise(&self) -> u64 {
        match self {
            ResourceType::CpuMicroseconds => 10_000_000,
            ResourceType::MemoryBytes => 2 * 1024 * 1024 * 1024,
            ResourceType::NetworkBytes => 1024 * 1024 * 1024,
            ResourceType::ConcurrentRequests => 1000,
            ResourceType::StorageBytes => 1024 * 1024 * 1024 * 1024,
        }
    }
}

impl fmt::Display for ResourceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResourceType::CpuMicroseconds => write!(f, "CPU"),
            ResourceType::MemoryBytes => write!(f, "Memory"),
            ResourceType::NetworkBytes => write!(f, "Network"),
            ResourceType::ConcurrentRequests => write!(f, "Concurrency"),
            ResourceType::StorageBytes => write!(f, "Storage"),
        }
    }
}

/// Error types for resource tracking
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Resource",
        link = "LEP-0004-laplace-interfaces-resource_domain_contracts"
    )
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceError {
    /// The supplied thread index exceeds the tracker's thread count.
    InvalidThreadId(ThreadId),
    /// The supplied resource index exceeds the tracker's resource count.
    InvalidResourceId(ResourceId),
    /// The thread already holds the resource; a double-acquire was attempted.
    AlreadyOwned {
        /// Thread that already owns the resource.
        thread: ThreadId,
        /// Resource being acquired a second time.
        resource: ResourceId,
    },
    /// The thread does not hold the resource; a release was attempted without a prior acquire.
    NotOwned {
        /// Thread that does not own the resource.
        thread: ThreadId,
        /// Resource for which the release was attempted.
        resource: ResourceId,
    },
    /// A circular wait chain was detected among threads.
    DeadlockDetected {
        /// Ordered sequence of thread IDs forming the deadlock cycle.
        cycle: Vec<ThreadId>,
    },
    /// A thread completed without releasing all held resources.
    ResourceLeak {
        /// Thread that exited while holding resources.
        thread: ThreadId,
        /// Resources still held by the thread at exit.
        held_resources: Vec<ResourceId>,
    },
}

impl fmt::Display for ResourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidThreadId(t) => write!(f, "Invalid thread ID: {}", t),
            Self::InvalidResourceId(r) => write!(f, "Invalid resource ID: {}", r),
            Self::AlreadyOwned { thread, resource } => {
                write!(f, "Thread {} already owns resource {}", thread, resource)
            }
            Self::NotOwned { thread, resource } => {
                write!(f, "Thread {} does not own resource {}", thread, resource)
            }
            Self::DeadlockDetected { cycle } => {
                write!(f, "Deadlock detected! Cycle: ")?;
                for (i, t) in cycle.iter().enumerate() {
                    if i > 0 {
                        write!(f, " -> ")?;
                    }
                    write!(f, "{}", t)?;
                }
                Ok(())
            }
            Self::ResourceLeak {
                thread,
                held_resources,
            } => {
                write!(f, "Thread {} finished with resources: ", thread)?;
                for (i, r) in held_resources.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", r)?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ResourceError {}

/// Result of a resource request operation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestResult {
    /// The resource was immediately granted to the requesting thread.
    Acquired,
    /// The resource is held by another thread; the requester must wait.
    Blocked,
}
