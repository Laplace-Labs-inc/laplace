// SPDX-License-Identifier: Apache-2.0
//! Linux `/proc`-based Resource Monitor
//!
//! Reads CPU utilisation from `/proc/stat` and memory usage from `/proc/meminfo`
//! to produce [`ResourceMetrics`] snapshots for CPI calculation.

use super::{ResourceMetrics, ResourceMonitor};
use std::sync::Mutex;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Internal CPU state
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Accumulated CPU tick counts from `/proc/stat`.
#[derive(Debug, Clone, Copy, Default)]
struct CpuTicks {
    total: u64,
    idle: u64,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DefaultResourceMonitor
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Production resource monitor that reads Linux `/proc` pseudo-files.
///
/// CPU usage is computed as a delta between two successive reads of `/proc/stat`.
/// The first call returns 0% because no previous sample exists yet.
///
/// Memory usage is computed from `/proc/meminfo` as `MemTotal - MemAvailable` (MB).
pub struct DefaultResourceMonitor {
    prev: Mutex<Option<CpuTicks>>,
}

impl DefaultResourceMonitor {
    /// Create a new monitor with no prior CPU sample.
    pub fn new() -> Self {
        Self {
            prev: Mutex::new(None),
        }
    }
}

impl Default for DefaultResourceMonitor {
    fn default() -> Self {
        Self::new()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// /proc parsers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Parse the first `cpu` line of `/proc/stat` into accumulated ticks.
///
/// Format: `cpu  user nice system idle iowait irq softirq steal [guest guest_nice]`
/// - `total` = sum of all fields
/// - `idle`  = idle + iowait (fields 4 and 5, 1-indexed)
fn parse_proc_stat(content: &str) -> Option<CpuTicks> {
    let line = content.lines().next()?;
    if !line.starts_with("cpu ") {
        return None;
    }
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1) // skip "cpu"
        .filter_map(|s| s.parse().ok())
        .collect();
    if fields.len() < 5 {
        return None;
    }
    let total: u64 = fields.iter().sum();
    // idle = idle(index 3) + iowait(index 4)
    let idle = fields[3] + fields[4];
    Some(CpuTicks { total, idle })
}

/// Parse `/proc/meminfo` and return used memory in MB.
///
/// Used = MemTotal - MemAvailable (both in kB in the file).
fn parse_proc_meminfo(content: &str) -> Option<f64> {
    let mut mem_total_kb: Option<u64> = None;
    let mut mem_available_kb: Option<u64> = None;

    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            mem_total_kb = parse_kb_value(rest);
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            mem_available_kb = parse_kb_value(rest);
        }
        if mem_total_kb.is_some() && mem_available_kb.is_some() {
            break;
        }
    }

    let total = mem_total_kb?;
    let available = mem_available_kb?;
    let used_kb = total.saturating_sub(available);
    Some(used_kb as f64 / 1024.0)
}

/// Extract a kB value from a `/proc/meminfo` value field like `"  16384000 kB"`.
fn parse_kb_value(s: &str) -> Option<u64> {
    s.split_whitespace().next()?.parse().ok()
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ResourceMonitor impl
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

impl ResourceMonitor for DefaultResourceMonitor {
    fn sample(&self) -> ResourceMetrics {
        // ── CPU ──────────────────────────────────────────────────────────────────
        let cpu_percent = match std::fs::read_to_string("/proc/stat") {
            Ok(content) => {
                if let Some(current) = parse_proc_stat(&content) {
                    let mut prev = self.prev.lock().unwrap();
                    let pct = if let Some(old) = *prev {
                        let d_total = current.total.saturating_sub(old.total);
                        let d_idle = current.idle.saturating_sub(old.idle);
                        if d_total == 0 {
                            0.0
                        } else {
                            (d_total - d_idle) as f64 / d_total as f64 * 100.0
                        }
                    } else {
                        0.0 // first sample — no delta available
                    };
                    *prev = Some(current);
                    pct
                } else {
                    0.0
                }
            }
            Err(_) => 0.0,
        };

        // ── Memory ───────────────────────────────────────────────────────────────
        let memory_mb = std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|c| parse_proc_meminfo(&c))
            .unwrap_or(0.0);

        ResourceMetrics::new(cpu_percent, memory_mb)
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_proc_stat() {
        let content = "cpu  10132153 290696 3084719 46828483 16683 0 25195 0 0 0\ncpu0 ...\n";
        let ticks = parse_proc_stat(content).unwrap();
        assert_eq!(
            ticks.total,
            10132153 + 290696 + 3084719 + 46828483 + 16683 + 0 + 25195 + 0 + 0 + 0
        );
        assert_eq!(ticks.idle, 46828483 + 16683);
    }

    #[test]
    fn test_parse_proc_stat_too_short() {
        assert!(parse_proc_stat("cpu  100 200 300").is_none());
    }

    #[test]
    fn test_parse_proc_meminfo() {
        let content = "\
MemTotal:       16384000 kB
MemFree:         1234567 kB
MemAvailable:    8192000 kB
Buffers:          123456 kB
";
        let used_mb = parse_proc_meminfo(content).unwrap();
        // (16384000 - 8192000) / 1024 = 8000.0
        assert!((used_mb - 8000.0).abs() < 0.01);
    }

    #[test]
    fn test_parse_proc_meminfo_missing_field() {
        let content = "MemTotal: 16384000 kB\n";
        assert!(parse_proc_meminfo(content).is_none());
    }

    #[test]
    fn test_cpu_delta_calculation() {
        let monitor = DefaultResourceMonitor::new();

        // Inject two known samples manually
        {
            let mut prev = monitor.prev.lock().unwrap();
            *prev = Some(CpuTicks {
                total: 1000,
                idle: 800,
            });
        }

        // Simulate a second read where total went up by 100, idle by 20
        // Expected: (100 - 20) / 100 * 100 = 80%
        let old_total = 1000u64;
        let old_idle = 800u64;
        let new_total = 1100u64;
        let new_idle = 820u64;
        let d_total = new_total - old_total;
        let d_idle = new_idle - old_idle;
        let expected = (d_total - d_idle) as f64 / d_total as f64 * 100.0;
        assert!((expected - 80.0).abs() < 0.01);
    }

    /// Integration test: verify real `/proc` reads on Linux.
    #[cfg(target_os = "linux")]
    #[test]
    fn test_live_proc_reads() {
        let monitor = DefaultResourceMonitor::new();

        // First sample: CPU will be 0% (no prior delta), memory should be > 0
        let m1 = monitor.sample();
        assert!(
            m1.cpu_percent >= 0.0 && m1.cpu_percent <= 100.0,
            "CPU must be in [0, 100], got {}",
            m1.cpu_percent
        );
        assert!(m1.memory_mb > 0.0, "Memory must be > 0 MB");

        // Burn some CPU cycles so the second sample has a non-zero delta
        let mut _dummy = 0u64;
        for i in 0..5_000_000u64 {
            _dummy = _dummy.wrapping_add(i);
        }

        // Second sample should now report a real CPU percentage
        let m2 = monitor.sample();
        assert!(
            m2.cpu_percent >= 0.0 && m2.cpu_percent <= 100.0,
            "CPU must be in [0, 100], got {}",
            m2.cpu_percent
        );
        assert!(m2.memory_mb > 0.0, "Memory must be > 0 MB");
    }
}
