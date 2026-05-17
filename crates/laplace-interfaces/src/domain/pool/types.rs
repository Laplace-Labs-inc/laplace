//! Pool domain type contracts

use serde::{Deserialize, Serialize};

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Storage strategy for tenant execution context
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Pool",
        link = "LEP-0006-laplace-interfaces-pool_strategy"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageStrategy {
    /// Standard FFI-based context storage with Protobuf serialization (~41.5µs)
    Standard,
    /// Turbo zero-copy shared memory context storage (<500ns)
    Turbo,
}

impl StorageStrategy {
    /// Returns the expected round-trip latency for this strategy in nanoseconds.
    ///
    /// - **Returns:** Approximate latency (nanoseconds); `Standard` ≈ 41 500 ns, `Turbo` < 500 ns.
    /// - **Ownership:** `self` is copied (cheap `Copy` type).
    pub fn expected_latency_ns(self) -> u64 {
        match self {
            Self::Standard => 41_500,
            Self::Turbo => 500,
        }
    }

    /// Returns `true` when this strategy avoids serialization via shared memory.
    ///
    /// - **Returns:** `true` only for [`StorageStrategy::Turbo`].
    /// - **Ownership:** `self` is copied (cheap `Copy` type).
    pub fn is_zero_copy(self) -> bool {
        matches!(self, Self::Turbo)
    }

    /// Returns a static, human-readable identifier string for this strategy.
    ///
    /// - **Returns:** `"Standard-FFI"` or `"Turbo-ZeroCopy"`.
    /// - **Ownership:** `self` is copied; returned value is a `'static` reference.
    pub fn name(self) -> &'static str {
        match self {
            Self::Standard => "Standard-FFI",
            Self::Turbo => "Turbo-ZeroCopy",
        }
    }
}

impl std::fmt::Display for StorageStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Pool health assessment categorization
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Pool",
        link = "LEP-0006-laplace-interfaces-pool_strategy"
    )
)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    /// All pool metrics are within acceptable thresholds; normal operation.
    Healthy,
    /// Pool is partially impaired but still serving requests.
    Degraded {
        /// Human-readable explanation of the degradation cause.
        reason: String,
    },
    /// Pool is non-functional and must be recovered before use.
    Unhealthy {
        /// Human-readable explanation of the failure cause.
        reason: String,
    },
}

impl HealthStatus {
    /// Returns `true` if the pool is operating normally.
    ///
    /// - **Returns:** `true` only for [`HealthStatus::Healthy`].
    /// - **Ownership:** `self` is immutably borrowed.
    pub fn is_healthy(&self) -> bool {
        matches!(self, Self::Healthy)
    }

    /// Returns `true` if the pool is partially impaired.
    ///
    /// - **Returns:** `true` only for [`HealthStatus::Degraded`].
    /// - **Ownership:** `self` is immutably borrowed.
    pub fn is_degraded(&self) -> bool {
        matches!(self, Self::Degraded { .. })
    }

    /// Returns `true` if the pool is non-functional.
    ///
    /// - **Returns:** `true` only for [`HealthStatus::Unhealthy`].
    /// - **Ownership:** `self` is immutably borrowed.
    pub fn is_unhealthy(&self) -> bool {
        matches!(self, Self::Unhealthy { .. })
    }

    /// Returns the failure or degradation reason string, if present.
    ///
    /// - **Returns:** `Some(&str)` for `Degraded` or `Unhealthy`; `None` for `Healthy`.
    /// - **Ownership:** `self` is immutably borrowed; returned reference has the same lifetime.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Healthy => None,
            Self::Degraded { reason } | Self::Unhealthy { reason } => Some(reason),
        }
    }
}
