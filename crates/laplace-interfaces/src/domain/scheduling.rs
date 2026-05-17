//! # Request Scheduling Priority Levels
//!
//! Foundational enumeration types for context classification and request scheduling.
//! Defines priority levels that determine execution order when resources are contended,
//! enabling the kernel scheduler to make intelligent preemption and queuing decisions.

use serde::{Deserialize, Serialize};
use std::fmt;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Request scheduling priority level
///
/// Used in multi-tenant contexts to determine execution order when system resources
/// are contended. The kernel scheduler uses this classification to make intelligent
/// decisions about preemption, queuing, and resource allocation.
///
/// Priority levels are totally ordered (can be compared with `<` and `>`), enabling
/// the scheduler to prioritize work systematically.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Scheduler",
        link = "LEP-0010-laplace-interfaces-scheduler_contracts"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum PriorityLevel {
    /// Lowest priority (0)
    ///
    /// Used for background tasks and batch operations with no time sensitivity.
    /// These operations are preempted by all higher priorities and only execute
    /// when the system is idle or during off-peak hours.
    Lowest = 0,

    /// Low priority (1)
    ///
    /// Used for non-interactive, deferred operations such as analytics aggregation,
    /// cache warming, and asynchronous notifications.
    Low = 1,

    /// Normal priority (2)
    ///
    /// Standard baseline for general-purpose workloads. This is the default priority
    /// for most standard requests that have no explicit time constraint.
    Normal = 2,

    /// High priority (3)
    ///
    /// Interactive user operations with expected response time constraints.
    /// This is the default priority for most user-facing requests. Operations
    /// at this level preempt Normal and lower priorities.
    High = 3,

    /// Critical priority (4)
    ///
    /// SLA-critical operations that require immediate execution with guaranteed
    /// response time. These operations preempt all lower-priority work and consume
    /// reserved system resources.
    Critical = 4,

    /// System critical (5)
    ///
    /// Reserved exclusively for kernel-internal operations and time-sensitive system
    /// maintenance tasks. This is the highest priority level and is only used by the
    /// kernel's scheduling logic, never by user-level request handlers.
    SystemCritical = 5,
}

impl PriorityLevel {
    /// Create a priority level from a u8 value with validation.
    ///
    /// # Arguments
    ///
    /// * `value` - Priority level in range [0, 5]
    ///
    /// # Returns
    ///
    /// `Some(PriorityLevel)` if value is in valid range, otherwise `None`.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// assert_eq!(PriorityLevel::from_u8(3), Some(PriorityLevel::High));
    /// assert_eq!(PriorityLevel::from_u8(6), None);
    /// ```
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(PriorityLevel::Lowest),
            1 => Some(PriorityLevel::Low),
            2 => Some(PriorityLevel::Normal),
            3 => Some(PriorityLevel::High),
            4 => Some(PriorityLevel::Critical),
            5 => Some(PriorityLevel::SystemCritical),
            _ => None,
        }
    }

    /// Get the priority level as a u8 value.
    ///
    /// # Returns
    ///
    /// Priority level as u8 in range [0, 5].
    #[inline]
    pub fn as_u8(&self) -> u8 {
        *self as u8
    }
}

#[allow(clippy::derivable_impls)]
impl Default for PriorityLevel {
    fn default() -> Self {
        PriorityLevel::Normal
    }
}

impl fmt::Display for PriorityLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PriorityLevel::Lowest => write!(f, "Lowest"),
            PriorityLevel::Low => write!(f, "Low"),
            PriorityLevel::Normal => write!(f, "Normal"),
            PriorityLevel::High => write!(f, "High"),
            PriorityLevel::Critical => write!(f, "Critical"),
            PriorityLevel::SystemCritical => write!(f, "SystemCritical"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_level_validation() {
        assert_eq!(PriorityLevel::from_u8(0), Some(PriorityLevel::Lowest));
        assert_eq!(PriorityLevel::from_u8(3), Some(PriorityLevel::High));
        assert_eq!(
            PriorityLevel::from_u8(5),
            Some(PriorityLevel::SystemCritical)
        );
        assert_eq!(PriorityLevel::from_u8(6), None);
    }

    #[test]
    fn priority_level_defaults() {
        assert_eq!(PriorityLevel::default(), PriorityLevel::Normal);
        assert_eq!(PriorityLevel::default().as_u8(), 2);
    }

    #[test]
    fn priority_level_display() {
        assert_eq!(PriorityLevel::High.to_string(), "High");
        assert_eq!(PriorityLevel::SystemCritical.to_string(), "SystemCritical");
    }

    #[test]
    fn priority_ordering() {
        assert!(PriorityLevel::Low < PriorityLevel::High);
        assert!(PriorityLevel::Critical > PriorityLevel::Normal);
        assert!(PriorityLevel::Lowest < PriorityLevel::SystemCritical);
    }

    #[test]
    fn priority_serialization() {
        use serde_json;

        let priority = PriorityLevel::Critical;
        let json = serde_json::to_string(&priority).unwrap();
        let deserialized: PriorityLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, PriorityLevel::Critical);
    }
}
