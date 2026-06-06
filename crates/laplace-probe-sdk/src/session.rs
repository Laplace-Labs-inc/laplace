// SPDX-License-Identifier: Apache-2.0
//! `ProbeSession` — 인메모리 이벤트 수집 + Ki-DPOR 실행.
//!
//! [GHOST CONSTRAINT]: `PROBE_SENDER`, `PROBE_THREAD_ID` thread-local은
//! OS 스레드 단위로 분리된다. `tokio::spawn()` 태스크에서 사용하면 동일 OS 스레드
//! 위의 다른 태스크와 공유된다 — 반드시 `std::thread::spawn()` + 개별 tokio 런타임 사용.
//!
//! [GHOST CONSTRAINT]: `ProbeSession`은 단일 `#[axiom_target]` 함수 = 단일 세션.
//! `run_verification_from()` 호출 후 재사용 불가.

use std::cell::Cell;
#[cfg(laplace_private_verification)]
use std::collections::hash_map::DefaultHasher;
#[cfg(laplace_private_verification)]
use std::collections::HashMap;
#[cfg(laplace_private_verification)]
use std::hash::{Hash, Hasher};
use std::sync::mpsc;

#[cfg(laplace_private_verification)]
use laplace_probe::ProbeEvent;
#[cfg(laplace_private_verification)]
use laplace_probe_common::decoder::DecodedProbeEvent;

use crate::license::load_axiom_max_depth;

// ── thread-local 선언 ──────────────────────────────────────────────────────────

thread_local! {
    /// 현재 OS 스레드에서 `ProbeEvent`를 수집하는 채널 송신단.
    /// 세션 외부(세션 미등록 스레드)에서는 None → no-op.
    #[cfg(laplace_private_verification)]
    static PROBE_SENDER: std::cell::RefCell<Option<mpsc::SyncSender<ProbeEvent>>> =
        const { std::cell::RefCell::new(None) };

    /// 현재 OS 스레드에 할당된 DPOR 가상 스레드 인덱스 (0-based).
    /// `#[axiom_target]` 생성 코드가 설정한다.
    static PROBE_THREAD_ID: Cell<u64> = const { Cell::new(0) };
}

// ── 공개 thread-local 설정 함수 ────────────────────────────────────────────────

/// 현재 OS 스레드의 `ProbeEvent` 송신단을 등록한다.
///
/// `#[axiom_target]` 생성 코드 내 `std::thread::spawn()` 클로저 진입 직후 호출.
#[cfg(laplace_private_verification)]
pub fn set_probe_sender(tx: mpsc::SyncSender<ProbeEvent>) {
    PROBE_SENDER.with(|s| *s.borrow_mut() = Some(tx));
}

#[cfg(not(laplace_private_verification))]
pub fn set_probe_sender<T>(_: mpsc::SyncSender<T>) {}

/// 현재 OS 스레드의 `ProbeEvent` 송신단을 초기화한다.
///
/// 테스트 또는 세션 정리 후 호출하여 thread-local을 정리한다.
#[cfg(laplace_private_verification)]
pub fn clear_probe_sender() {
    PROBE_SENDER.with(|s| *s.borrow_mut() = None);
}

#[cfg(not(laplace_private_verification))]
pub fn clear_probe_sender() {}

/// 현재 OS 스레드에 DPOR 가상 스레드 인덱스를 할당한다.
///
/// `#[axiom_target]` 생성 코드 내 `std::thread::spawn()` 클로저 진입 직후 호출.
pub fn set_probe_thread_id(id: u64) {
    PROBE_THREAD_ID.with(|c| c.set(id));
}

/// 현재 OS 스레드의 DPOR 가상 스레드 인덱스를 읽는다.
///
/// `TrackedMutex::lock()`이 내부적으로 호출한다.
pub fn current_thread_id() -> u64 {
    PROBE_THREAD_ID.with(Cell::get)
}

/// `ProbeEvent`를 현재 OS 스레드의 수집 채널로 전송한다.
///
/// `TrackedMutex`/`TrackedGuard`가 내부적으로 호출한다.
/// 채널 미등록(세션 외부) 시 no-op.
#[cfg(laplace_private_verification)]
pub fn emit(event: ProbeEvent) {
    PROBE_SENDER.with(|s| {
        if let Some(tx) = s.borrow().as_ref() {
            let _ = tx.send(event.clone());
        }
    });

    #[cfg(all(feature = "cloud", laplace_private_verification))]
    if let Some(client) = cloud::GLOBAL_PROBE_CLIENT.get() {
        if let Some(raw) = cloud::probe_event_to_raw(&event) {
            client.emit(raw);
        }
    }
}

#[cfg(not(laplace_private_verification))]
pub fn emit(_: ()) {}

// ── ProbeSessionConfig ─────────────────────────────────────────────────────────

/// `ProbeSession` 설정.
#[derive(Debug, Clone)]
pub struct ProbeSessionConfig {
    /// Ki-DPOR 최대 탐색 깊이.
    ///
    /// [GHOST CONSTRAINT]: JWT `limits.axiom_max_depth`가 있으면 이 값보다 우선.
    /// `load_axiom_max_depth()` 결과가 Some이면 그 값을 사용한다.
    pub max_depth: usize,
    /// 버그 발견 시 .ard 파일 저장 여부.
    pub write_ard: bool,
    /// .ard 파일 저장 디렉터리.
    pub output_dir: String,
}

impl Default for ProbeSessionConfig {
    fn default() -> Self {
        // 우선순위: JWT tier limit > laplace.toml > 하드코딩 500
        let max_depth = load_axiom_max_depth() // [1] JWT (SaaS tier hard-cap)
            .or_else(|| crate::config::load_toml_max_depth()) // [2] laplace.toml
            .unwrap_or(500); // [3] fallback
        Self {
            max_depth,
            write_ard: true,
            output_dir: ".".to_string(),
        }
    }
}

// ── VerifyResult ───────────────────────────────────────────────────────────────

/// Ki-DPOR 검증 결과.
#[cfg(laplace_private_verification)]
pub struct VerifyResult {
    pub verdict: laplace_axiom::oracle::OracleVerdict,
    pub thread_count: usize,
    pub resource_count: usize,
    pub events_collected: usize,
}

#[cfg(laplace_private_verification)]
impl VerifyResult {
    /// 버그가 발견될 것을 기대하는 테스트에서 사용.
    ///
    /// Ki-DPOR가 `BugFound`를 반환하면 통과. `Clean`이면 panic (탐지 실패).
    ///
    /// # Panics
    ///
    /// Panics if no bug is found (i.e., DPOR returned `Clean` unexpectedly).
    pub fn assert_bug(self) {
        use laplace_axiom::oracle::OracleVerdict;
        match self.verdict {
            OracleVerdict::BugFound {
                ard_path,
                description,
            } => {
                println!(
                    "✅ Axiom: BUG DETECTED — {} threads, {} resources, {} events",
                    self.thread_count, self.resource_count, self.events_collected,
                );
                println!("   description: {description}");
                println!("   ard: {ard_path}");
            }
            OracleVerdict::Clean => {
                panic!(
                    "❌ Axiom: Expected BugFound but got CLEAN\n  \
                     DPOR가 교착을 탐지하지 못했습니다. 락 획득 순서 또는 이벤트 수집을 확인하세요."
                );
            }
        }
    }

    /// `cargo test`에서 사용. 버그 발견 시 `panic!`으로 테스트 실패.
    ///
    /// # Panics
    ///
    /// Panics if a bug is found during verification.
    pub fn assert_clean(self) {
        use laplace_axiom::oracle::OracleVerdict;
        match self.verdict {
            OracleVerdict::Clean => {
                println!(
                    "✅ Axiom: CLEAN — {} threads, {} resources, {} events, depth cap applied",
                    self.thread_count, self.resource_count, self.events_collected,
                );
            }
            OracleVerdict::BugFound {
                ard_path,
                description,
            } => {
                panic!("❌ Axiom: BUG FOUND\n  description: {description}\n  ard: {ard_path}");
            }
        }
    }
}

// ── 변환 헬퍼 — verify.rs 로직 동일 복제 ─────────────────────────────────────

/// 자원 이름 문자열 → 안정적 `u64` 주소 (`DefaultHasher`, 동일 프로세스 내 결정적).
///
/// [GHOST CONSTRAINT]: 동일 자원명은 항상 동일 해시를 반환해야 한다.
/// `ResourceRegistry`가 이름 해시를 `ResourceId`로 고정 매핑하므로 변경 금지.
#[cfg(laplace_private_verification)]
pub(crate) fn resource_name_to_addr(name: &str) -> u64 {
    let mut h = DefaultHasher::new();
    name.hash(&mut h);
    h.finish()
}

/// `ProbeEvent` → `DecodedProbeEvent` 변환.
///
/// verify.rs `probe_event_to_decoded()`와 동일 매핑 규칙.
/// - `LockAcquired` → `LockAcquire` (Request 생성)
/// - `LockReleased` → `LockRelease` (Release 생성)
/// - 나머지 → None (DPOR 무관)
#[cfg(laplace_private_verification)]
#[allow(clippy::cast_possible_truncation)]
fn probe_event_to_decoded(event: &ProbeEvent, timestamp_ns: u64) -> Option<DecodedProbeEvent> {
    match event {
        ProbeEvent::LockAcquired {
            thread_id,
            resource,
        } => Some(DecodedProbeEvent::LockAcquire {
            // thread_id is bounded by MAX_AXIOM_THREADS = 8 < u32::MAX
            tid: *thread_id as u32,
            timestamp_ns,
            mutex_addr: resource_name_to_addr(resource),
            contention_ns: 0,
        }),
        ProbeEvent::LockReleased {
            thread_id,
            resource,
        } => Some(DecodedProbeEvent::LockRelease {
            // thread_id is bounded by MAX_AXIOM_THREADS = 8 < u32::MAX
            tid: *thread_id as u32,
            timestamp_ns,
            mutex_addr: resource_name_to_addr(resource),
        }),
        ProbeEvent::ThreadBlocked {
            thread_id,
            blocked_on,
        } => Some(DecodedProbeEvent::LockContention {
            // thread_id is bounded by MAX_AXIOM_THREADS = 8 < u32::MAX
            tid: *thread_id as u32,
            timestamp_ns,
            mutex_addr: resource_name_to_addr(blocked_on),
        }),
        ProbeEvent::RwLockReadAcquired {
            thread_id,
            resource,
        } => Some(DecodedProbeEvent::RwLockReadAcquire {
            tid: *thread_id as u32,
            timestamp_ns,
            rwlock_addr: resource_name_to_addr(resource),
        }),
        ProbeEvent::RwLockReadReleased {
            thread_id,
            resource,
        } => Some(DecodedProbeEvent::RwLockReadRelease {
            tid: *thread_id as u32,
            timestamp_ns,
            rwlock_addr: resource_name_to_addr(resource),
        }),
        ProbeEvent::RwLockWriteAcquired {
            thread_id,
            resource,
        } => Some(DecodedProbeEvent::RwLockWriteAcquire {
            tid: *thread_id as u32,
            timestamp_ns,
            rwlock_addr: resource_name_to_addr(resource),
        }),
        ProbeEvent::RwLockWriteReleased {
            thread_id,
            resource,
        } => Some(DecodedProbeEvent::RwLockWriteRelease {
            tid: *thread_id as u32,
            timestamp_ns,
            rwlock_addr: resource_name_to_addr(resource),
        }),
        ProbeEvent::AtomicLoad {
            thread_id,
            resource,
        } => Some(DecodedProbeEvent::AtomicLoad {
            tid: *thread_id as u32,
            timestamp_ns,
            addr: resource_name_to_addr(resource),
        }),
        ProbeEvent::AtomicStore {
            thread_id,
            resource,
        } => Some(DecodedProbeEvent::AtomicStore {
            tid: *thread_id as u32,
            timestamp_ns,
            addr: resource_name_to_addr(resource),
        }),
        ProbeEvent::AtomicRmw {
            thread_id,
            resource,
        } => Some(DecodedProbeEvent::AtomicRmw {
            tid: *thread_id as u32,
            timestamp_ns,
            addr: resource_name_to_addr(resource),
        }),
        ProbeEvent::SemaphoreAcquired {
            thread_id,
            resource,
        } => Some(DecodedProbeEvent::SemaphoreAcquire {
            tid: *thread_id as u32,
            timestamp_ns,
            sem_addr: resource_name_to_addr(resource),
        }),
        ProbeEvent::SemaphoreReleased {
            thread_id,
            resource,
        } => Some(DecodedProbeEvent::SemaphoreRelease {
            tid: *thread_id as u32,
            timestamp_ns,
            sem_addr: resource_name_to_addr(resource),
        }),
        _ => None,
    }
}

// ── build_step_programs — verify.rs 로직 동일 복제 ───────────────────────────

/// Type alias for per-thread DPOR step programs.
#[cfg(laplace_private_verification)]
type DporStepProgram = Vec<
    Vec<(
        laplace_axiom::dpor::Operation,
        laplace_core::domain::resource::ResourceId,
    )>,
>;

/// `ProbeEvent` 스트림 → 스레드별 DPOR step 프로그램으로 변환.
///
/// verify.rs `build_step_programs_from_probe_events()`와 동일 로직.
/// laplace-cli는 library crate가 아니므로 복제 필수.
#[cfg(laplace_private_verification)]
fn build_step_programs(
    probe_events: &[ProbeEvent],
) -> (
    DporStepProgram,
    Vec<usize>,
    laplace_probe_common::axiom_adapter::AxiomStepBuilder,
) {
    use laplace_axiom::dpor::Operation;
    use laplace_core::domain::resource::ResourceId;
    use laplace_probe_common::axiom_adapter::{AxiomEvent, AxiomOp, AxiomStepBuilder};

    let mut builder = AxiomStepBuilder::new();
    let mut ts: u64 = 1_000_000_000;

    let mut axiom_events: Vec<AxiomEvent> = Vec::new();
    for event in probe_events {
        if let Some(decoded) = probe_event_to_decoded(event, ts) {
            if let Some(axiom_event) = builder.process(&decoded) {
                axiom_events.push(axiom_event);
            }
            ts += 100;
        }
    }

    let num_threads = builder.thread_registry().len().max(1);
    let mut per_thread: Vec<Vec<(Operation, ResourceId)>> = vec![Vec::new(); num_threads];

    for event in &axiom_events {
        if let AxiomEvent::Step(step) = event {
            if step.thread < num_threads {
                per_thread[step.thread].push((
                    match step.op {
                        AxiomOp::Request => Operation::Request,
                        AxiomOp::Release => Operation::Release,
                        AxiomOp::SharedRequest => Operation::SharedRequest,
                        AxiomOp::SharedRelease => Operation::SharedRelease,
                        AxiomOp::Read => Operation::Read,
                        AxiomOp::Write => Operation::Write,
                        AxiomOp::ReadWrite => Operation::ReadWrite,
                    },
                    ResourceId::new(step.resource),
                ));
            }
        }
    }

    let mut compacted: Vec<Vec<(Operation, ResourceId)>> = Vec::new();
    let mut dpor_to_axiom: Vec<usize> = Vec::new();
    for (axiom_id, steps) in per_thread.into_iter().enumerate() {
        if !steps.is_empty() {
            dpor_to_axiom.push(axiom_id);
            compacted.push(steps);
        }
    }

    (compacted, dpor_to_axiom, builder)
}

#[cfg(laplace_private_verification)]
fn build_symbol_table(
    events: &[ProbeEvent],
    resources: &laplace_probe_common::axiom_adapter::ResourceRegistry,
) -> HashMap<String, String> {
    let mut symbol_table = HashMap::new();

    for event in events {
        let Some(resource_name) = event.resource_name() else {
            continue;
        };
        let resource_addr = resource_name_to_addr(resource_name);
        let Some(resource_id) = resources.get(resource_addr) else {
            continue;
        };
        symbol_table
            .entry(format!("r{resource_id}"))
            .or_insert_with(|| resource_name.to_string());
    }

    symbol_table
}

// ── run_verification_from — 공개 진입점 ──────────────────────────────────────

/// 수집된 `ProbeEvent` 목록을 받아 Ki-DPOR 검증을 실행한다.
///
/// `#[axiom_target]` 생성 코드의 마지막 단계에서 호출한다.
///
/// # 파라미터
///
/// - `events`: 모든 스레드가 완료된 후 `mpsc` 채널에서 수집한 `ProbeEvent` 목록
/// - `target_name`: 검증 대상 식별자 (ARD 헤더에 기록됨)
/// - `config`: `max_depth` / `write_ard` / `output_dir` 설정
#[cfg(laplace_private_verification)]
pub fn run_verification_from(
    events: &[ProbeEvent],
    target_name: &str,
    config: &ProbeSessionConfig,
) -> VerifyResult {
    use laplace_axiom::oracle::{AxiomOracle, OracleConfig};
    use laplace_axiom::simulation::TwinSimulatorBuilder;
    use laplace_core::domain::resource::{ResourceId, ThreadId};

    let events_collected = events.len();
    let (compacted_steps, _dpor_to_axiom, step_builder) = build_step_programs(events);

    let num_active_threads = compacted_steps.len().max(1);
    let num_resources = step_builder.resource_registry().len().max(1);
    let symbol_table = build_symbol_table(events, step_builder.resource_registry());

    let mut simulator = TwinSimulatorBuilder::new()
        .cores(num_active_threads.max(2))
        .scheduler_threads(num_active_threads.max(2))
        .finalize()
        .build();

    simulator.run_until_idle();

    // 범용 메모리 초기화 — 데모 특화 값 없음
    for i in 0..num_active_threads {
        if let Err(e) = simulator.memory_mut().write(
            laplace_core::domain::memory::CoreId::new(0),
            laplace_core::domain::memory::Address::new(i),
            laplace_core::domain::memory::Value::new(1),
        ) {
            tracing::warn!("SDK: memory init write failed at {i}: {e}");
        }
        simulator.run_until_idle();
    }

    let oracle = AxiomOracle::new(OracleConfig {
        num_threads: num_active_threads,
        num_resources,
        max_depth: config.max_depth,
        output_dir: config.output_dir.clone(),
        write_ard: config.write_ard,
        symbol_table,
        ..OracleConfig::default()
    });

    let verdict = oracle.run_exhaustive(
        target_name,
        &mut simulator,
        config.max_depth,
        |thread: ThreadId, pc: usize| -> Option<(laplace_axiom::dpor::Operation, ResourceId)> {
            compacted_steps
                .get(thread.as_usize())
                .and_then(|steps| steps.get(pc))
                .copied()
        },
        |_sim: &mut laplace_axiom::simulation::TwinSimulator| -> Option<String> {
            None // Phase 1: no-op invariant checker
        },
    );

    VerifyResult {
        verdict,
        thread_count: num_active_threads,
        resource_count: num_resources,
        events_collected,
    }
}

#[cfg(all(test, laplace_private_verification))]
mod tests {
    use super::*;

    #[test]
    fn symbol_table_uses_session_local_resource_ids() {
        let events = vec![
            ProbeEvent::LockAcquired {
                thread_id: 0,
                resource: "mutex_a".to_string(),
            },
            ProbeEvent::LockAcquired {
                thread_id: 0,
                resource: "mutex_b".to_string(),
            },
            ProbeEvent::LockReleased {
                thread_id: 0,
                resource: "mutex_b".to_string(),
            },
            ProbeEvent::LockReleased {
                thread_id: 0,
                resource: "mutex_a".to_string(),
            },
        ];

        let (_steps, _threads, builder) = build_step_programs(&events);
        let symbol_table = build_symbol_table(&events, builder.resource_registry());

        assert_eq!(symbol_table.get("r0"), Some(&"mutex_a".to_string()));
        assert_eq!(symbol_table.get("r1"), Some(&"mutex_b".to_string()));
        assert_eq!(symbol_table.len(), 2);
    }
}

// ── 클라우드 관측 경로 (feature = "cloud") ──────────────────────────────────────

#[cfg(all(feature = "cloud", laplace_private_verification))]
mod cloud {
    use std::sync::OnceLock;

    use crate::client::{ProbeClient, ProbeClientConfig, RawProbeEvent};
    use laplace_probe::ProbeEvent;

    pub(super) static GLOBAL_PROBE_CLIENT: OnceLock<ProbeClient> = OnceLock::new();

    /// 클라우드 관측 모드를 초기화한다.
    ///
    /// 이후 emit() 시 로컬 DPOR 채널과 클라우드 WebSocket 양쪽으로 전송된다.
    /// 두 번 호출 시 두 번째는 무시(OnceLock).
    ///
    /// [Ghost Constraint]: tokio 런타임 내에서 호출 필수.
    pub async fn init_cloud_probe(config: ProbeClientConfig) -> anyhow::Result<()> {
        if GLOBAL_PROBE_CLIENT.get().is_some() {
            return Ok(()); // 이미 초기화됨
        }
        let client = ProbeClient::connect(config).await?;
        let _ = GLOBAL_PROBE_CLIENT.set(client);
        Ok(())
    }

    /// ProbeEvent를 RawProbeEvent로 변환 (cloud 전송용 최소 필드만).
    pub fn probe_event_to_raw(event: &ProbeEvent) -> Option<RawProbeEvent> {
        let mut raw: RawProbeEvent = bytemuck::Zeroable::zeroed();
        match event {
            ProbeEvent::LockAcquired { thread_id, .. } => {
                raw.event_type = 4; // LockAcquired = 4
                raw.tid = *thread_id as u32;
            }
            ProbeEvent::LockReleased { thread_id, .. } => {
                raw.event_type = 5; // LockRelease = 5
                raw.tid = *thread_id as u32;
            }
            ProbeEvent::ThreadBlocked { thread_id, .. } => {
                raw.event_type = 6; // LockContention = 6
                raw.tid = *thread_id as u32;
            }
            _ => return None,
        }
        raw.timestamp_ns = (laplace_core::domain::now_ms().max(0) as u64) * 1_000_000;
        Some(raw)
    }
}

#[cfg(all(feature = "cloud", laplace_private_verification))]
pub use cloud::init_cloud_probe;
