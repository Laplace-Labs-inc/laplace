// SPDX-License-Identifier: Apache-2.0
//! Shared telemetry event contract.

use super::entropy::ContextId;
use super::telemetry_domain::TelemetryDomain;

/// A discrete telemetry event emitted by Laplace runtime components.
#[non_exhaustive]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum TelemetryEvent {
    /// A log-level error message from any subsystem.
    LogError(String),
    /// The DPOR scheduler backtracked at the given context.
    DporBacktrack(ContextId),
    /// A context changed its operational state.
    StateChanged(ContextId, String),
    /// Captured API request/response trace.
    ApiTrace {
        /// HTTP method.
        method: String,
        /// Request path.
        path: String,
        /// Serialized payload summary.
        payload: String,
        /// Whether the trace represents an error response.
        is_error: bool,
    },
    /// Axiom Oracle detected a liveness violation.
    AxiomViolation(String),
    /// Axiom Oracle completed exhaustive search without violation.
    AxiomClean,
    /// Rust panic captured by the telemetry hook.
    Panic {
        /// Redacted panic message.
        message: String,
        /// Optional redacted source location.
        location: Option<String>,
    },
    /// External telemetry sink/backend failure.
    ExternalSinkError {
        /// Sink name.
        sink: String,
        /// Redacted error text.
        error: String,
    },
}

impl TelemetryEvent {
    /// Routing domain for this event.
    ///
    /// External-domain events are eligible for opt-in upload to remote sinks
    /// (Sentry/GlitchTip): user-facing errors, panics, and self-observability
    /// of external sinks themselves. Everything else is routed Internal —
    /// consumed only by in-process subscribers like the Hollywood TUI.
    pub fn domain(&self) -> TelemetryDomain {
        match self {
            Self::LogError(_) | Self::Panic { .. } | Self::ExternalSinkError { .. } => {
                TelemetryDomain::External
            }
            _ => TelemetryDomain::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TelemetryDomain, TelemetryEvent};
    use crate::domain::entropy::ContextId;

    #[test]
    fn domain_routes_external_sink_errors_to_external() {
        let event = TelemetryEvent::ExternalSinkError {
            sink: "sentry".to_owned(),
            error: "queue full".to_owned(),
        };
        assert_eq!(event.domain(), TelemetryDomain::External);
    }

    #[test]
    fn domain_routes_panics_to_external() {
        let event = TelemetryEvent::Panic {
            message: "panic".to_owned(),
            location: None,
        };
        assert_eq!(event.domain(), TelemetryDomain::External);
    }

    #[test]
    fn domain_routes_log_errors_to_external() {
        let event = TelemetryEvent::LogError("subsystem failure".to_owned());
        assert_eq!(event.domain(), TelemetryDomain::External);
    }

    #[test]
    fn domain_routes_internal_events_to_internal() {
        let backtrack = TelemetryEvent::DporBacktrack(ContextId::new(0));
        assert_eq!(backtrack.domain(), TelemetryDomain::Internal);

        let clean = TelemetryEvent::AxiomClean;
        assert_eq!(clean.domain(), TelemetryDomain::Internal);
    }
}
