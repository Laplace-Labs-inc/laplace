// SPDX-License-Identifier: Apache-2.0
//! Object-safe telemetry sink contract.

use super::{TelemetryDomain, TelemetryEvent};

/// Object-safe sink for telemetry event delivery.
pub trait TelemetrySink: Send + Sync {
    /// Stable sink name for diagnostics.
    fn name(&self) -> &'static str;

    /// Domain accepted by this sink.
    fn domain(&self) -> TelemetryDomain;

    /// Handle one event synchronously.
    ///
    /// Implementations that perform I/O should enqueue work and return quickly.
    fn handle(&self, event: &TelemetryEvent);

    /// Flush queued events if the sink buffers asynchronously.
    fn flush(&self);
}
