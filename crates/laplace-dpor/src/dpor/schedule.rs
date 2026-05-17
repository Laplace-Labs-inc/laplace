// SPDX-License-Identifier: Apache-2.0
//! Schedule — Serializable Bug-Schedule Extraction Type
//!
//! A `Schedule` is a snapshot of an execution path produced by DPOR schedulers.

use super::classic::StepRecord;
/// A captured execution schedule.
///
/// Produced by [`super::classic::DporScheduler::extract_schedule`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Schedule {
    /// Ordered list of execution steps forming the defect-triggering interleaving.
    pub steps: Vec<StepRecord>,

    /// Optional violation text for compatibility with higher-level consumers.
    pub violation: Option<String>,
}
