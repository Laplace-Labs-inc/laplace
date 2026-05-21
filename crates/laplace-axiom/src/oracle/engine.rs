// SPDX-License-Identifier: Apache-2.0
//! VerdictEngine — ARD dump on violation (Non-Public, SDK Blacklist)
//!
//! This module contains the Schedule → ARD forensic report conversion logic.
//! It is gated behind `feature = "engine"` and excluded from public SDK releases.

#![cfg(feature = "verification")]

use super::{OracleConfig, OracleVerdict};
use laplace_core::domain::journal::{ArdHeader, ForensicFrame, ForensicWindow};
use laplace_ki_dpor::Schedule;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

pub(super) struct VerdictEngine {
    config: OracleConfig,
}

impl VerdictEngine {
    pub(super) fn new(config: OracleConfig) -> Self {
        Self { config }
    }

    /// Convert a DPOR [`Schedule`] into an [`ArdReport`] and write it to disk.
    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(
            layer = "30_Axiom_Oracle",
            link = "LEP-0011-laplace-axiom-oracle_forensics_and_bmc"
        )
    )]
    pub(super) fn dump(&self, target_id: &str, schedule: &Schedule) -> OracleVerdict {
        let violation_desc = schedule
            .violation
            .as_ref()
            .map(|v| format!("{v:?}"))
            .unwrap_or_else(|| "UnknownViolation".to_string());

        // Build the forensic window from the schedule's step records.
        let mut window = ForensicWindow::new();
        let steps = &schedule.steps;
        let total = steps.len();

        for (i, step) in steps.iter().enumerate() {
            let frame = ForensicFrame::new(
                0, // corrected by ForensicWindow
                format!("t{}", step.thread.as_usize()),
                format!("{:?}", step.operation),
                format!("resource=r{}", step.resource.as_usize()),
                if i + 1 == total {
                    violation_desc.clone()
                } else {
                    "ok".to_string()
                },
                vec![format!("dpor_depth={}", step.depth)],
            );
            if i + 1 == total {
                window.set_error(frame);
            } else {
                window.push_pre(frame);
            }
        }

        // Snapshot ref is a seed-derived placeholder (Sled integration: future).
        let snapshot_ref = format!("seed:{:#018x}", self.config.axiom_seed);
        let mut header = ArdHeader::new(self.config.axiom_seed, target_id, snapshot_ref);
        header.symbol_table = self.config.symbol_table.clone();
        let report = window.into_report(header);

        let ard_path = if self.config.write_ard {
            let ts = laplace_core::domain::now_ms();
            let filename = format!("bug_report_{ts}.ard");
            let path = if self.config.output_dir == "." {
                filename
            } else {
                format!("{}/{}", self.config.output_dir, filename)
            };

            if let Err(e) = save_ard(&report, &path) {
                tracing::error!(error = %e, path = %path, "Oracle: failed to write ARD file");
            } else {
                tracing::info!(path = %path, "Oracle: ARD written");
            }
            path
        } else {
            tracing::debug!("Oracle: ARD write suppressed (write_ard = false)");
            "<suppressed>".to_string()
        };

        OracleVerdict::BugFound {
            ard_path,
            description: violation_desc,
        }
    }
}

// ── ARD 파일 IO 헬퍼 (feature = "engine" 전용) ──────────────────────────────

/// ArdReport를 JSON으로 직렬화하여 파일에 쓴다.
///
/// [Ghost Constraint]: feature = "engine" 전용. 공개 SDK 빌드에 미포함.
#[cfg(feature = "engine")]
pub fn save_ard(
    report: &laplace_core::domain::journal::ArdReport,
    path: &str,
) -> std::io::Result<()> {
    let json = report
        .to_json()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, json)
}

/// 파일에서 ArdReport를 로드한다.
///
/// [Ghost Constraint]: feature = "engine" 전용. 공개 SDK 빌드에 미포함.
#[cfg(feature = "engine")]
pub fn load_ard(path: &str) -> std::io::Result<laplace_core::domain::journal::ArdReport> {
    let content = std::fs::read_to_string(path)?;
    laplace_core::domain::journal::ArdReport::from_json(&content)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}
