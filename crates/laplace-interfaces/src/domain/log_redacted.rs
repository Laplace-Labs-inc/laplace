// SPDX-License-Identifier: Apache-2.0
//! Redacted wrapper for values that must never be exposed in logs.

use std::fmt;

/// Wrapper whose `Debug` and `Display` output is always `[REDACTED]`.
#[repr(transparent)]
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct LogRedacted<T>(pub T);

impl<T> fmt::Debug for LogRedacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl<T> fmt::Display for LogRedacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[cfg(test)]
mod tests {
    use super::LogRedacted;

    #[test]
    fn debug_and_display_are_redacted() {
        let value = LogRedacted("token".to_owned());
        assert_eq!(format!("{value:?}"), "[REDACTED]");
        assert_eq!(value.to_string(), "[REDACTED]");
    }
}
