// SPDX-License-Identifier: Apache-2.0
//! Telemetry sink routing domains.

/// Domain for routing telemetry events to sinks.
#[repr(u8)]
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TelemetryDomain {
    /// Internal engine/runtime telemetry.
    Internal = 0,
    /// External sink or backend telemetry.
    External = 1,
}

#[cfg(test)]
mod tests {
    use super::TelemetryDomain;

    #[test]
    fn telemetry_domain_is_stable_u8() {
        assert_eq!(TelemetryDomain::Internal as u8, 0);
        assert_eq!(TelemetryDomain::External as u8, 1);
    }
}
