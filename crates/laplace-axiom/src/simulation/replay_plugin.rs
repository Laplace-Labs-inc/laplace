//! Schedule Replay Plugin — deterministic execution replay via captured schedules.
//!
//! Feed a [`ScheduleReplayPlugin`] into [`TwinSimulatorBuilder::inject`] to reproduce
//! the exact thread interleaving recorded during a DPOR fault-finding run.

use super::hooks::VirtualEnvPlugin;
use crate::dpor::Schedule;
use laplace_core::domain::memory::{MemoryBackend, MemoryOp};
use laplace_core::domain::scheduler::ThreadId;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Replays a captured [`Schedule`] by injecting thread IDs step by step.
///
/// On each call to [`inject_schedule_override`](VirtualEnvPlugin::inject_schedule_override)
/// the plugin returns the next step's `ThreadId` from the schedule, advancing an internal
/// cursor. Once the schedule is exhausted `None` is returned and the simulator falls back
/// to its default event ordering.
///
/// # Example
///
/// ```rust,ignore
/// let schedule = runner.run(&mut sim, 200, op_provider).expect("violation found");
/// let replay_plugin = ScheduleReplayPlugin::new(schedule);
///
/// let mut replay_sim = TwinSimulatorBuilder::new()
///     .cores(4)
///     .scheduler_threads(4)
///     .finalize()
///     .inject(replay_plugin)
///     .build();
///
/// replay_sim.run_until_idle();
/// ```
pub struct ScheduleReplayPlugin {
    schedule: Schedule,
    cursor: usize,
}

impl ScheduleReplayPlugin {
    /// Create a new plugin that will replay `schedule` from the beginning.
    pub fn new(schedule: Schedule) -> Self {
        Self {
            schedule,
            cursor: 0,
        }
    }

    /// Returns `true` when all scheduled steps have been injected.
    pub fn is_exhausted(&self) -> bool {
        self.cursor >= self.schedule.steps.len()
    }

    /// Number of steps remaining in the schedule.
    pub fn remaining(&self) -> usize {
        self.schedule.steps.len().saturating_sub(self.cursor)
    }

    /// Reference to the underlying schedule.
    pub fn schedule(&self) -> &Schedule {
        &self.schedule
    }
}

impl VirtualEnvPlugin for ScheduleReplayPlugin {
    /// Return the next step's thread from the captured schedule.
    ///
    /// Advances the cursor on each call. Returns `None` once the schedule is exhausted.
    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(
            layer = "30_Axiom_Simulation",
            link = "LEP-0010-laplace-axiom-digital_twin_and_typestate"
        )
    )]
    fn inject_schedule_override(&mut self, _tick: u64) -> Option<ThreadId> {
        let step = self.schedule.steps.get(self.cursor)?;
        self.cursor += 1;
        // Convert resource::ThreadId → scheduler::ThreadId (both wrap usize).
        Some(ThreadId::new(step.thread.as_usize()))
    }

    /// No memory injections — replay only controls thread ordering.
    fn inject_memory_op(
        &mut self,
        _tick: u64,
        _memory: &mut dyn MemoryBackend,
    ) -> Option<MemoryOp> {
        None
    }

    fn name(&self) -> &'static str {
        "ScheduleReplayPlugin"
    }
}
