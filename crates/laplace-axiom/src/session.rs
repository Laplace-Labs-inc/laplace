// SPDX-License-Identifier: Apache-2.0
//! VerificationSession — CQS-compliant Oracle entry point (K-3)
//!
//! 진입부/엔진 분리 설계:
//!   `docs/Laplace-Labs-Docs/concepts/architecture/verification-session-cqs.md`
//!
//! Command: run(&mut self, config) — 반환 없음
//! Query:   verdict/stats/is_complete — &self, side-effect 없음

use laplace_interfaces::AxiomConfig;
use std::fmt;

use crate::oracle::{OracleConfig, OracleVerdict};

// ── VerificationConfig ────────────────────────────────────────────────────────

/// VerificationSession::run()에 전달하는 불변 설정값.
/// Command 호출 전 완전히 구성되어야 한다.
pub struct VerificationConfig {
    /// ARD 헤더 및 로그 레이블.
    pub target_id: String,
    pub num_threads: usize,
    pub num_resources: usize,
    pub max_depth: usize,
    pub axiom_seed: u64,
    /// ARD 출력 경로. feature = "engine" 가 없으면 무시된다.
    pub output_dir: String,
    /// ARD 파일 쓰기 여부. feature = "engine" 가 없으면 무시된다.
    pub write_ard: bool,
    /// (thread_idx, pc) → (Operation, ResourceId) 매핑. HarnessConfig에서 주입.
    pub op_provider: Box<
        dyn FnMut(
            laplace_core::domain::resource::ThreadId,
            usize,
        ) -> Option<(
            laplace_dpor::Operation,
            laplace_core::domain::resource::ResourceId,
        )>,
    >,
}

impl fmt::Debug for VerificationConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerificationConfig")
            .field("target_id", &self.target_id)
            .field("num_threads", &self.num_threads)
            .field("num_resources", &self.num_resources)
            .field("max_depth", &self.max_depth)
            .field("axiom_seed", &self.axiom_seed)
            .field("output_dir", &self.output_dir)
            .field("write_ard", &self.write_ard)
            .field("op_provider", &"<FnMut>")
            .finish()
    }
}

impl VerificationConfig {
    /// HarnessConfig로부터 VerificationConfig를 생성한다.
    #[cfg(feature = "twin")]
    pub fn from_harness(
        h: &laplace_harness::registry::HarnessConfig,
        max_depth: usize,
        write_ard: bool,
    ) -> Self {
        Self {
            target_id: format!("harness::{}", h.name),
            num_threads: h.num_threads,
            num_resources: h.num_resources,
            max_depth,
            axiom_seed: AxiomConfig::default().default_seed,
            output_dir: ".".to_string(),
            write_ard,
            op_provider: Box::new(h.op_provider),
        }
    }
}

// ── ExplorationStats ──────────────────────────────────────────────────────────

/// Query 전용 탐색 통계. DporStats의 공개 프로젝션.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExplorationStats {
    pub explored_states: usize,
    pub max_depth_reached: usize,
    pub violation_found: bool,
}

// ── VerificationSession ───────────────────────────────────────────────────────

/// CQS-compliant Oracle 진입부.
///
/// K-3 Ghost Constraint: Send + 'static 유지 필수 (tokio task 이동).
/// SovereignSession impl은 laplace-kernel 의존 추가 시 별도 태스크에서 처리.
pub struct VerificationSession {
    verdict: Option<OracleVerdict>,
    stats: ExplorationStats,
}

impl VerificationSession {
    pub fn new() -> Self {
        Self {
            verdict: None,
            stats: ExplorationStats::default(),
        }
    }

    // ── Command ──────────────────────────────────────────────────────────────

    /// DPOR 탐색을 동기적으로 완전 수행한다.
    ///
    /// 반환값 없음. 완료 후 verdict()/stats()로 결과를 조회한다.
    /// 두 번 호출 시 이전 결과를 덮어쓴다.
    ///
    /// [Ghost Constraint]: async fn 금지. tokio::task::spawn_blocking 불필요.
    #[cfg(all(feature = "twin", feature = "verification"))]
    pub fn run(&mut self, mut config: VerificationConfig) {
        use crate::oracle::AxiomOracle;
        use crate::simulation::TwinSimulatorBuilder;
        use laplace_core::domain::memory::{Address, CoreId, Value};

        let oracle = AxiomOracle::new(OracleConfig {
            num_threads: config.num_threads,
            num_resources: config.num_resources,
            max_depth: config.max_depth,
            axiom_seed: config.axiom_seed,
            output_dir: config.output_dir.clone(),
            write_ard: config.write_ard,
        });

        let mut simulator = TwinSimulatorBuilder::new()
            .cores(config.num_threads)
            .scheduler_threads(config.num_threads)
            .finalize()
            .build();

        for i in 0..config.num_threads {
            simulator.run_until_idle();
            let _ = simulator.memory_mut().write(
                CoreId::new(0),
                Address::new(i),
                Value::new(i as u64 + 1),
            );
        }
        simulator.run_until_idle();

        let verdict = oracle.run_exhaustive(
            &config.target_id,
            &mut simulator,
            config.max_depth,
            &mut *config.op_provider,
            |_sim| None,
        );

        let stats = ExplorationStats {
            explored_states: laplace_core::domain::telemetry::GlobalTelemetry::metrics()
                .explored_states() as usize,
            max_depth_reached: config.max_depth,
            violation_found: matches!(verdict, OracleVerdict::BugFound { .. }),
        };

        self.stats = stats;
        self.verdict = Some(verdict);
    }

    // ── Queries ───────────────────────────────────────────────────────────────

    /// 탐색 완료 후 OracleVerdict를 반환한다. run() 이전에는 None.
    pub fn verdict(&self) -> Option<&OracleVerdict> {
        self.verdict.as_ref()
    }

    /// 탐색 통계를 반환한다. run() 이전에는 Default.
    pub fn stats(&self) -> ExplorationStats {
        self.stats
    }

    /// verdict가 채워져 있으면 true.
    pub fn is_complete(&self) -> bool {
        self.verdict.is_some()
    }
}

impl Default for VerificationSession {
    fn default() -> Self {
        Self::new()
    }
}
