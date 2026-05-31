// SPDX-License-Identifier: Apache-2.0
//! Shared telemetry configuration contract.

use super::{LogRedacted, TelemetryDomain};

/// Shared telemetry sink configuration.
#[repr(C, align(8))]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TelemetryConfig {
    /// Sentry-compatible DSN. Redacted in logs.
    pub dsn: LogRedacted<String>,
    /// Runtime environment name.
    pub environment: String,
    /// Release version reported to the sink.
    pub release_version: String,
    /// Event sampling rate from 0.0 to 1.0.
    pub sample_rate: f32,
    /// Retention days. LQ-14 fixes this to 180.
    pub retention_days: u32,
    /// Separate telemetry opt-in. Defaults false.
    pub opt_in: bool,
    /// Sink routing domain.
    pub domain: TelemetryDomain,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            dsn: LogRedacted(String::new()),
            environment: "development".to_owned(),
            release_version: "unknown".to_owned(),
            sample_rate: 0.0,
            retention_days: 180,
            opt_in: false,
            domain: TelemetryDomain::External,
        }
    }
}

impl TelemetryConfig {
    /// Validate the config without exposing secret values.
    pub fn is_valid(&self) -> bool {
        (0.0..=1.0).contains(&self.sample_rate) && self.retention_days == 180
    }
}

#[cfg(test)]
mod tests {
    use super::TelemetryConfig;

    #[test]
    fn defaults_to_opt_out() {
        let config = TelemetryConfig::default();
        assert!(!config.opt_in);
        assert_eq!(config.retention_days, 180);
        assert!(config.is_valid());
    }
}
