// SPDX-License-Identifier: Apache-2.0
//! CPI (Cost-Performance Index) 계산 모듈
//!
//! RPS(초당 처리량)와 투입 자원(CPU/Memory)을 바탕으로 실시간 가성비를 계산한다.
//!
//! # 공식
//!
//! ```text
//! CPI = RPS / (cpu_percent + memory_mb × 10.0)
//! ```
//!
//! - 분모가 0에 가까울 경우(자원 소모 없음) 0.0 반환
//! - Memory에 10배 가중치를 부여하여 메모리 압박이 점수에 크게 반영되도록 설계

/// RPS, CPU 사용률, 메모리 사용량으로 CPI 점수를 계산한다.
///
/// # Arguments
///
/// * `rps` — 초당 처리 요청 수 (Requests Per Second)
/// * `cpu_percent` — CPU 사용률 (0.0 ~ 100.0)
/// * `memory_mb` — 사용 중인 메모리 (MB)
///
/// # Returns
///
/// CPI 점수. 높을수록 자원 대비 처리량이 우수함을 의미한다.
/// 분모가 0.001 미만이면 자원 소모가 없는 것으로 간주하여 0.0 반환.
pub fn calculate_cpi(rps: f64, cpu_percent: f64, memory_mb: f64) -> f64 {
    let denominator = cpu_percent + memory_mb * 10.0;
    if denominator < 0.001 {
        0.0
    } else {
        rps / denominator
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    /// CPU 50%, Memory 60MB, RPS 1000 → CPI ≈ 1.538
    #[test]
    fn test_cpi_expected_value() {
        // denominator = 50 + 60*10 = 650
        // cpi = 1000 / 650 ≈ 1.5384...
        let cpi = calculate_cpi(1000.0, 50.0, 60.0);
        assert!(
            (cpi - 1.538).abs() < 0.001,
            "Expected ~1.538, got {:.4}",
            cpi
        );
    }

    /// 메모리가 늘어나면 CPI가 급격히 하락해야 한다 (10배 가중치 검증).
    #[test]
    fn test_memory_weight_causes_steep_cpi_drop() {
        let rps = 1000.0;
        let cpu = 10.0;

        let cpi_low_mem = calculate_cpi(rps, cpu, 10.0); // denom = 10 + 100 = 110
        let cpi_high_mem = calculate_cpi(rps, cpu, 100.0); // denom = 10 + 1000 = 1010

        assert!(
            cpi_high_mem < cpi_low_mem,
            "CPI must decrease as memory increases"
        );

        // 메모리 10배 증가(10MB→100MB) 시 CPI가 약 9배 이상 하락
        let ratio = cpi_low_mem / cpi_high_mem;
        assert!(
            ratio > 9.0,
            "Memory 10x increase should cause >9x CPI drop due to 10x weight, got ratio={:.2}",
            ratio
        );
    }

    /// 자원 소모가 0에 수렴할 때 0.0 반환 (0 나누기 방지).
    #[test]
    fn test_zero_resource_returns_zero() {
        assert_eq!(calculate_cpi(1000.0, 0.0, 0.0), 0.0);
        assert_eq!(calculate_cpi(0.0, 0.0, 0.0), 0.0);
    }

    /// RPS가 0이면 CPI도 0.
    #[test]
    fn test_zero_rps_returns_zero() {
        let cpi = calculate_cpi(0.0, 50.0, 100.0);
        assert_eq!(cpi, 0.0);
    }

    /// CPU만 높고 Memory=0인 경우의 CPI 계산.
    #[test]
    fn test_cpu_only_resource() {
        // denominator = 100.0 + 0.0 = 100.0
        let cpi = calculate_cpi(500.0, 100.0, 0.0);
        assert!((cpi - 5.0).abs() < 1e-9, "Expected 5.0, got {}", cpi);
    }
}
