//! DPOR Runner — orchestrator that coordinates [`KiDporScheduler`] with simulators.
//!
//! `DporRunner` drives the DPOR A\*-based state space exploration loop.
//! Simulator integration (TwinSimulator) is provided via laplace-axiom's extension methods.

use super::ki_scheduler::KiDporScheduler;

/// Orchestrates DPOR state space exploration with a scheduler.
///
/// The runner exposes the inner scheduler for integration with external simulators.
/// Simulator integration methods are provided in laplace-axiom.
pub struct DporRunner {
    scheduler: KiDporScheduler,
}

impl DporRunner {
    /// Create a new `DporRunner` owning `scheduler`.
    pub fn new(scheduler: KiDporScheduler) -> Self {
        Self { scheduler }
    }

    /// Borrow the inner scheduler for inspection.
    pub fn scheduler(&self) -> &KiDporScheduler {
        &self.scheduler
    }

    /// Borrow the inner scheduler mutably for integration with external simulators.
    pub fn scheduler_mut(&mut self) -> &mut KiDporScheduler {
        &mut self.scheduler
    }
}
