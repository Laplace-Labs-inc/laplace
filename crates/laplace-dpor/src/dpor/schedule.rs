//! Schedule — Serializable Bug-Schedule Extraction Type
//!
//! A `Schedule` is a snapshot of an execution path produced by DPOR schedulers
//! when a concurrency defect is identified. It bundles the ordered list of steps
//! with the detected liveness violation (if any) to support offline serialization
//! and deterministic replay.

use super::classic::StepRecord;
use super::ki_scheduler::LivenessViolation;

/// A captured execution schedule, optionally paired with a detected violation.
///
/// Produced by [`super::classic::DporScheduler::extract_schedule`] and
/// [`super::ki_scheduler::KiDporScheduler::extract_schedule`].
/// Can be serialized to JSON for storage and later replayed via
/// `ScheduleReplayPlugin` (future P1 integration).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Schedule {
    /// Ordered list of execution steps forming the defect-triggering interleaving.
    pub steps: Vec<StepRecord>,

    /// Liveness violation detected at the end of this schedule, if any.
    pub violation: Option<LivenessViolation>,
}
