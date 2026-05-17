// SPDX-License-Identifier: Apache-2.0
//! eBPF Lock Hunt — `pthread_mutex_lock/unlock` uprobe로 Lock 이벤트를 캡처하고
//! AB-BA cycle 분석으로 deadlock을 탐지한다.
//!
//! # 사용법
//!
//! 1. eBPF 커널 프로그램 빌드:
//!    ```bash
//!    cd crates/probe/laplace-probe-ebpf && cargo +nightly build --release
//!    ```
//!
//! 2. 타겟 프로세스 실행:
//!    ```bash
//!    cargo build -p ebpf-lock-test
//!    ./target/debug/ebpf-lock-test &
//!    TEST_PID=$!
//!    ```
//!
//! 3. Lock hunt 실행 (root 필요):
//!    ```bash
//!    sudo ./target/debug/lock-hunt --pid $TEST_PID --duration 15
//!    ```

use aya::{
    maps::{HashMap, RingBuf},
    programs::{TracePoint, UProbe},
    Ebpf,
};
use aya_log::EbpfLogger;
use bytemuck::{from_bytes, Pod, Zeroable};
use clap::Parser;
use tokio::io::unix::AsyncFd;

// ── eBPF 오브젝트 ─────────────────────────────────────────────────────────────

/// Path to the compiled eBPF object file.
///
/// Build with:
///   cd crates/probe/laplace-probe-ebpf
///   cargo +nightly build --release
const EBPF_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../laplace-probe-ebpf/target/bpfel-unknown-none/release/laplace-probe-ebpf"
));

/// Runtime-aligned copy of the embedded eBPF ELF.
/// Heap-allocated to satisfy object crate's alignment requirement.
fn ebpf_object_aligned() -> Vec<u8> {
    // Vec<u8> is heap-allocated and thus naturally aligned to at least 8 bytes.
    EBPF_BYTES.to_vec()
}

// ── 인라인 타입 정의 ─────────────────────────────────────────────────────────
//
// laplace-probe-common은 laplace-probe-sdk → laplace-axiom → laplace-probe 순환 때문에
// laplace-probe의 직접 의존성으로 추가할 수 없다.
// RawProbeEvent와 이벤트 타입 상수를 여기에 직접 정의한다.

/// Lock 획득 완료 이벤트 discriminant.
/// [GHOST CONSTRAINT]: laplace-probe-common ProbeEventType::LockAcquire = 3
const EVENT_LOCK_ACQUIRE: u8 = 3;

/// Lock 해제 이벤트 discriminant.
/// [GHOST CONSTRAINT]: laplace-probe-common ProbeEventType::LockRelease = 5
const EVENT_LOCK_RELEASE: u8 = 5;

/// Lock 획득 완료 이벤트 discriminant (contention_ns 포함).
/// [GHOST CONSTRAINT]: laplace-probe-common ProbeEventType::LockAcquired = 4
const EVENT_LOCK_ACQUIRED: u8 = 4;

/// Lock 경합 이벤트 discriminant (커널 PI deadlock 탐지 등).
/// [GHOST CONSTRAINT]: laplace-probe-common ProbeEventType::LockContention = 6
const EVENT_LOCK_CONTENTION: u8 = 6;

/// Shared kernel/user-space event structure — laplace-probe-common::RawProbeEvent의 미러.
///
/// // [ABI_GUARD]: FFI Boundary — 커널 ring buffer에서 읽는 128바이트 repr(C) 구조체.
/// GHOST CONSTRAINT: 필드 순서/크기/패딩 변경 금지. laplace-probe-common과 항상 동기화.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct RawProbeEvent {
    timestamp_ns: u64,   // 8
    tid: u32,            // 4
    pid: u32,            // 4
    event_type: u8,      // 1
    l4_proto: u8,        // 1
    status_code: u16,    // 2
    _pad0: u32,          // 4
    resource_id: u64,    // 8
    peer_addr: u64,      // 8
    peer_port: u32,      // 4
    local_port: u32,     // 4
    payload_hash: u64,   // 8
    payload_len: u32,    // 4
    operation_hash: u32, // 4
    latency_ns: u64,     // 8
    _pad1: u64,          // 8
    correlation_id: u64, // 8
    cpu_id: u32,         // 4
    depth: u32,          // 4
    comm: [u8; 16],      // 16
    parent_tid: u64,     // 8
    _reserved: u64,      // 8
}
// Total: 8+4+4+1+1+2+4+8+8+4+4+8+4+4+8+8+8+4+4+16+8+8 = 128 bytes

const _: () = assert!(
    core::mem::size_of::<RawProbeEvent>() == 128,
    "RawProbeEvent mirror must be exactly 128 bytes — sync with laplace-probe-common"
);

// ── CLI ──────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "lock-hunt",
    about = "eBPF futex tracepoint — Lamport WFG deadlock detector"
)]
struct Args {
    /// 타겟 프로세스 PID
    #[arg(long)]
    pid: u32,

    /// libc.so 경로 (pthread_mutex_lock/unlock 심볼 위치)
    #[arg(long, default_value = "/lib/x86_64-linux-gnu/libc.so.6")]
    libc: String,

    /// 수집 시간 (초). 0이면 Ctrl-C까지 대기.
    #[arg(long, default_value = "10")]
    duration: u64,

    /// pthread uprobe 활성화 — C/C++ 바이너리 타겟에서만 필요.
    /// 기본 비활성화 — Rust std::sync::Mutex는 pthread를 우회하므로 uprobe가
    /// LockAcquire(완료) vs LockAcquire(요청) semantic 충돌을 일으킨다.
    #[arg(long, default_value_t = false)]
    enable_pthread_uprobe: bool,
}

// ── main ─────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // ── 1. eBPF 로드 ─────────────────────────────────────────────────────────
    let ebpf_data = ebpf_object_aligned();
    eprintln!("[lock-hunt] eBPF object size: {} bytes", ebpf_data.len());
    let mut bpf = match Ebpf::load(&ebpf_data) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[lock-hunt] Ebpf::load failed: {e:?}");
            return Err(e.into());
        }
    };

    if let Err(e) = EbpfLogger::init(&mut bpf) {
        eprintln!("[lock-hunt] eBPF logger init warning (non-fatal): {e}");
    }

    // ── 2. TARGET_PIDS 맵에 타겟 PID 등록 ────────────────────────────────────
    {
        let mut target_pids: HashMap<_, u32, u8> =
            HashMap::try_from(bpf.map_mut("TARGET_PIDS").unwrap())?;
        target_pids.insert(args.pid, 1, 0)?;
    }
    println!("[lock-hunt] Registered target PID: {}", args.pid);

    // ── 3. sched_switch tracepoint 부착 (기존 인프라 유지) ───────────────────
    {
        let sched: &mut TracePoint = bpf.program_mut("sched_switch").unwrap().try_into()?;
        sched.load()?;
        sched.attach("sched", "sched_switch")?;
    }

    // ── 4~6. pthread uprobe (선택적) ─────────────────────────────────────────
    // 기본 비활성화. Rust std::sync::Mutex는 pthread를 우회하므로 uprobe가
    // glibc 내부 mutex와 Rust Mutex 이벤트의 semantic 충돌을 일으킨다.
    // C/C++ 바이너리 타겟에서만 --enable-pthread-uprobe로 활성화.
    if args.enable_pthread_uprobe {
        {
            let lock_enter: &mut UProbe =
                bpf.program_mut("mutex_lock_enter").unwrap().try_into()?;
            lock_enter.load()?;
            lock_enter.attach(
                Some("pthread_mutex_lock"),
                0,
                &args.libc,
                Some(args.pid as i32),
            )?;
        }
        {
            let lock_exit: &mut UProbe = bpf.program_mut("mutex_lock_exit").unwrap().try_into()?;
            lock_exit.load()?;
            lock_exit.attach(
                Some("pthread_mutex_lock"),
                0,
                &args.libc,
                Some(args.pid as i32),
            )?;
        }
        {
            let unlock: &mut UProbe = bpf.program_mut("mutex_unlock").unwrap().try_into()?;
            unlock.load()?;
            unlock.attach(
                Some("pthread_mutex_unlock"),
                0,
                &args.libc,
                Some(args.pid as i32),
            )?;
        }
        println!(
            "[lock-hunt] Attached 3 pthread uprobe hooks to {}. (C/C++ mode)",
            args.libc
        );
    } else {
        println!(
            "[lock-hunt] Skipping pthread uprobe (Rust mode). \
             Use --enable-pthread-uprobe for C/C++ tracing."
        );
    }

    // ── 7. sys_enter_futex tracepoint 부착 ──────────────────────────────────
    {
        let futex_enter: &mut TracePoint =
            bpf.program_mut("sys_enter_futex").unwrap().try_into()?;
        futex_enter.load()?;
        futex_enter.attach("syscalls", "sys_enter_futex")?;
    }

    // ── 8. sys_exit_futex tracepoint 부착 ───────────────────────────────────
    {
        let futex_exit: &mut TracePoint = bpf.program_mut("sys_exit_futex").unwrap().try_into()?;
        futex_exit.load()?;
        futex_exit.attach("syscalls", "sys_exit_futex")?;
    }

    println!("[lock-hunt] Attached sys_enter_futex + sys_exit_futex tracepoints.");

    // ── 7. Ring buffer 소비 + 이벤트 수집 ───────────────────────────────────
    let ring_buf = RingBuf::try_from(bpf.map_mut("EVENTS").unwrap())?;
    let mut async_fd = AsyncFd::new(ring_buf)?;
    let mut collected: Vec<RawProbeEvent> = Vec::new();

    if args.duration > 0 {
        let deadline =
            tokio::time::Instant::now() + tokio::time::Duration::from_secs(args.duration);

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            tokio::select! {
                result = async_fd.readable_mut() => {
                    let mut guard = result?;
                    drain_ring_buffer(guard.get_inner_mut(), &mut collected);
                    guard.clear_ready();
                }
                _ = tokio::time::sleep(remaining) => break,
            }
        }
    } else {
        loop {
            let mut guard = async_fd.readable_mut().await?;
            drain_ring_buffer(guard.get_inner_mut(), &mut collected);
            guard.clear_ready();
        }
    }

    // ── 8. 결과 분석 ─────────────────────────────────────────────────────────
    println!(
        "\n[lock-hunt] Collection complete. {} raw events.",
        collected.len()
    );
    analyze_events(&collected);

    Ok(())
}

// ── 헬퍼 함수 ────────────────────────────────────────────────────────────────

/// Ring buffer에서 이벤트를 모두 꺼내 `events`에 추가하고 Lock 이벤트를 실시간 출력한다.
fn drain_ring_buffer(ring: &mut RingBuf<&mut aya::maps::MapData>, events: &mut Vec<RawProbeEvent>) {
    while let Some(item) = ring.next() {
        let data: &[u8] = &item;
        if data.len() < core::mem::size_of::<RawProbeEvent>() {
            continue;
        }
        let event: &RawProbeEvent = from_bytes(&data[..core::mem::size_of::<RawProbeEvent>()]);

        match event.event_type {
            EVENT_LOCK_ACQUIRE => {
                println!(
                    "  [LockAcquire ] tid={} uaddr=0x{:016x} op=0x{:x}",
                    event.tid, event.resource_id, event.operation_hash
                );
            }
            EVENT_LOCK_ACQUIRED => {
                println!(
                    "  [LockAcquired] tid={} uaddr=0x{:016x} contention={}ns op=0x{:x}",
                    event.tid, event.resource_id, event.latency_ns, event.operation_hash
                );
            }
            EVENT_LOCK_RELEASE => {
                println!(
                    "  [LockRelease ] tid={} uaddr=0x{:016x} op=0x{:x}",
                    event.tid, event.resource_id, event.operation_hash
                );
            }
            EVENT_LOCK_CONTENTION if event.status_code == 35 => {
                println!(
                    "  [!!! KERNEL PI DEADLOCK !!!] tid={} uaddr=0x{:016x} contention={}ns",
                    event.tid, event.resource_id, event.latency_ns
                );
            }
            _ => {} // sched_switch 등은 실시간 출력 생략
        }

        events.push(*event);
    }
}

/// 수집된 이벤트에서 AB-BA cycle을 탐지한다.
fn analyze_events(raw_events: &[RawProbeEvent]) {
    // 진단: event_type별 카운트. futex tracepoint가 호출되는지 확인.
    let mut by_type: [usize; 256] = [0; 256];
    for e in raw_events {
        by_type[e.event_type as usize] += 1;
    }
    println!("[lock-hunt] Event breakdown:");
    println!("  SchedSwitch   (7):  {}", by_type[7]);
    println!(
        "  LockAcquire   (3):  {}  (requests — enter)",
        by_type[EVENT_LOCK_ACQUIRE as usize]
    );
    println!(
        "  LockAcquired  (4):  {}  (completed — exit)",
        by_type[EVENT_LOCK_ACQUIRED as usize]
    );
    println!(
        "  LockRelease   (5):  {}",
        by_type[EVENT_LOCK_RELEASE as usize]
    );
    println!(
        "  LockContention(6):  {}",
        by_type[EVENT_LOCK_CONTENTION as usize]
    );

    // futex tracepoint origin 이벤트는 operation_hash != 0
    // uprobe origin 이벤트는 operation_hash == 0
    let futex_origin: usize = raw_events
        .iter()
        .filter(|e| {
            (e.event_type == EVENT_LOCK_ACQUIRE || e.event_type == EVENT_LOCK_RELEASE)
                && e.operation_hash != 0
        })
        .count();
    let uprobe_origin: usize = raw_events
        .iter()
        .filter(|e| {
            (e.event_type == EVENT_LOCK_ACQUIRE || e.event_type == EVENT_LOCK_RELEASE)
                && e.operation_hash == 0
        })
        .count();
    println!("  └─ futex origin (op != 0):  {}", futex_origin);
    println!("  └─ uprobe origin (op == 0): {}", uprobe_origin);

    let lock_events: Vec<&RawProbeEvent> = raw_events
        .iter()
        .filter(|e| e.event_type == EVENT_LOCK_ACQUIRE || e.event_type == EVENT_LOCK_RELEASE)
        .collect();

    println!("[lock-hunt] Lock events: {}", lock_events.len());

    if lock_events.is_empty() {
        println!("[lock-hunt] No lock events — check --libc path and target PID.");
        return;
    }

    let mut mutexes: Vec<u64> = lock_events.iter().map(|e| e.resource_id).collect();
    mutexes.sort_unstable();
    mutexes.dedup();
    println!("[lock-hunt] Unique mutexes: {}", mutexes.len());

    let mut tids: Vec<u32> = lock_events.iter().map(|e| e.tid).collect();
    tids.sort_unstable();
    tids.dedup();
    println!("[lock-hunt] Unique threads: {}", tids.len());

    // 스레드별 Lock ordering 출력 (최초 32 이벤트만)
    for &tid in &tids {
        let thread_evts: Vec<&&RawProbeEvent> =
            lock_events.iter().filter(|e| e.tid == tid).collect();
        println!("\n  Thread {} ({} events):", tid, thread_evts.len());
        for e in thread_evts.iter().take(32) {
            let op = if e.event_type == EVENT_LOCK_ACQUIRE {
                "ACQUIRE"
            } else {
                "RELEASE"
            };
            println!("    {} mutex=0x{:016x}", op, e.resource_id);
        }
    }

    analyze_deadlock_wfg(raw_events);
}

// ── Lamport clock + WFG Deadlock Detector ────────────────────────────────────

/// tid → (uaddr, lamport_time): 완료되지 않은 acquire 요청
type PendingMap = std::collections::HashMap<u32, (u64, u64)>;
/// uaddr → (tid, lamport_time): 현재 보유 중인 스레드
type HoldersMap = std::collections::HashMap<u64, (u32, u64)>;

/// 이벤트에 Lamport clock을 부여하고 pending/held 상태를 재구성한다.
fn reconstruct_wait_state(events: &[RawProbeEvent]) -> (PendingMap, HoldersMap) {
    use std::collections::HashMap;

    let mut sorted: Vec<&RawProbeEvent> = events.iter().collect();
    sorted.sort_by_key(|e| e.timestamp_ns);

    let mut per_tid: HashMap<u32, u64> = HashMap::new();
    let mut lamport_global: u64 = 0;

    let mut pending: HashMap<u32, (u64, u64)> = HashMap::new();
    let mut holders: HashMap<u64, (u32, u64)> = HashMap::new();

    for ev in &sorted {
        // Lamport tick
        let tid_clock = per_tid.entry(ev.tid).or_insert(0);
        *tid_clock = (*tid_clock).max(lamport_global) + 1;
        let lamport_time = *tid_clock;
        lamport_global = lamport_global.max(lamport_time);

        match ev.event_type {
            EVENT_LOCK_ACQUIRE => {
                // 요청 시작 — 아직 보유하지 않음
                pending.insert(ev.tid, (ev.resource_id, lamport_time));
            }
            EVENT_LOCK_ACQUIRED => {
                // 획득 완료 — pending 해제 후 holder 등록
                pending.remove(&ev.tid);
                holders.insert(ev.resource_id, (ev.tid, lamport_time));
            }
            EVENT_LOCK_RELEASE => {
                // 해제 — 현재 holder가 이 tid일 때만 제거
                let is_owner = holders
                    .get(&ev.resource_id)
                    .map(|&(owner, _)| owner == ev.tid)
                    .unwrap_or(false);
                if is_owner {
                    holders.remove(&ev.resource_id);
                }
            }
            _ => {}
        }
    }

    (pending, holders)
}

/// WFG를 구축하고 DFS로 cycle을 탐지한다.
///
/// Returns `Some(Vec<tid>)` — cycle을 구성하는 tid 순서, `None` — cycle 없음.
fn detect_wfg_cycle(pending: &PendingMap, holders: &HoldersMap) -> Option<Vec<u32>> {
    use std::collections::{HashMap, HashSet};

    // WFG 구축: waiter_tid → Vec<holder_tid>
    let mut edges: HashMap<u32, Vec<u32>> = HashMap::new();
    for (&waiter, &(uaddr, _)) in pending {
        if let Some(&(owner, _)) = holders.get(&uaddr) {
            if owner != waiter {
                edges.entry(waiter).or_default().push(owner);
            }
        }
    }

    // 모든 노드 수집 (dedup)
    let nodes: HashSet<u32> = edges
        .keys()
        .copied()
        .chain(edges.values().flatten().copied())
        .collect();

    // DFS with gray/black sets (back-edge detection)
    let mut visiting: HashSet<u32> = HashSet::new();
    let mut visited: HashSet<u32> = HashSet::new();
    let mut parent: HashMap<u32, u32> = HashMap::new();

    fn dfs_visit(
        node: u32,
        edges: &std::collections::HashMap<u32, Vec<u32>>,
        visiting: &mut std::collections::HashSet<u32>,
        visited: &mut std::collections::HashSet<u32>,
        parent: &mut std::collections::HashMap<u32, u32>,
    ) -> Option<(u32, u32)> {
        visiting.insert(node);
        if let Some(neighbors) = edges.get(&node) {
            for &next in neighbors {
                if visiting.contains(&next) {
                    return Some((node, next)); // back-edge
                }
                if !visited.contains(&next) {
                    parent.insert(next, node);
                    if let Some(edge) = dfs_visit(next, edges, visiting, visited, parent) {
                        return Some(edge);
                    }
                }
            }
        }
        visiting.remove(&node);
        visited.insert(node);
        None
    }

    for start in nodes {
        if !visiting.contains(&start) && !visited.contains(&start) {
            if let Some((from, back_to)) =
                dfs_visit(start, &edges, &mut visiting, &mut visited, &mut parent)
            {
                // cycle 재구성: back_to → ... → from → back_to
                let mut cycle = vec![back_to];
                let mut cur = from;
                while cur != back_to {
                    cycle.push(cur);
                    match parent.get(&cur) {
                        Some(&p) => cur = p,
                        None => break,
                    }
                }
                cycle.reverse();
                return Some(cycle);
            }
        }
    }

    None
}

/// Tier 2: holders=0인 AB-BA 암묵 추론.
///
/// Rust Mutex fast path(CAS)는 futex를 발생시키지 않으므로 holders 맵이 비어 있다.
/// pending 스레드가 정확히 2개이고 서로 다른 uaddr을 대기 중이면 AB-BA 교착 패턴이다.
fn detect_implicit_ab_ba(pending: &PendingMap) -> Option<Vec<u32>> {
    if pending.len() != 2 {
        return None;
    }
    let mut iter = pending.iter();
    let (&tid_a, &(uaddr_a, _)) = iter.next()?;
    let (&tid_b, &(uaddr_b, _)) = iter.next()?;
    if uaddr_a != uaddr_b {
        Some(vec![tid_a, tid_b])
    } else {
        None
    }
}

/// Tier 3: 전체 활성 스레드 블로킹 탐지.
///
/// 2개 이상의 활성 스레드가 모두 pending 상태이면 시스템 전체가 교착되었다고 판단한다.
fn detect_total_blocking(
    pending: &PendingMap,
    active_tids: &std::collections::HashSet<u32>,
) -> Option<Vec<u32>> {
    if pending.len() < 2 || active_tids.len() < 2 {
        return None;
    }
    if active_tids.iter().all(|tid| pending.contains_key(tid)) {
        Some(pending.keys().copied().collect())
    } else {
        None
    }
}

/// WFG 기반 deadlock 탐지 + 결과 출력. 기존 `detect_ab_ba_cycle`을 대체한다.
fn analyze_deadlock_wfg(raw_events: &[RawProbeEvent]) {
    let (pending, holders) = reconstruct_wait_state(raw_events);

    let active_tids: std::collections::HashSet<u32> = raw_events
        .iter()
        .filter(|e| {
            matches!(
                e.event_type,
                EVENT_LOCK_ACQUIRE
                    | EVENT_LOCK_ACQUIRED
                    | EVENT_LOCK_RELEASE
                    | EVENT_LOCK_CONTENTION
            )
        })
        .map(|e| e.tid)
        .collect();

    println!("\n[lock-hunt] === Lamport WFG Deadlock Analysis ===");
    println!("  Pending acquire requests: {}", pending.len());
    for (&tid, &(uaddr, lamport)) in &pending {
        println!(
            "    tid={} waiting on uaddr=0x{:016x} (lamport={})",
            tid, uaddr, lamport
        );
    }
    println!("  Held resources: {}", holders.len());
    for (&uaddr, &(tid, lamport)) in &holders {
        println!(
            "    uaddr=0x{:016x} held by tid={} (lamport={})",
            uaddr, tid, lamport
        );
    }

    if let Some(cycle) = detect_wfg_cycle(&pending, &holders) {
        println!("\n[lock-hunt] ⚠  DEADLOCK DETECTED — Tier 1: WFG cycle (confirmed)");
        println!("  Cycle: {:?}", cycle);
        for i in 0..cycle.len() {
            let waiter = cycle[i];
            let owner = cycle[(i + 1) % cycle.len()];
            if let Some(&(uaddr, _)) = pending.get(&waiter) {
                println!(
                    "    Thread {} waits on 0x{:016x} held by Thread {}",
                    waiter, uaddr, owner
                );
            }
        }
    } else if let Some(cycle) = detect_implicit_ab_ba(&pending) {
        println!(
            "\n[lock-hunt] ⚠  DEADLOCK DETECTED — Tier 2: implicit AB-BA \
             (holders=0, Rust fast path)"
        );
        println!("  Implicated threads: {:?}", cycle);
        for &tid in &cycle {
            if let Some(&(uaddr, _)) = pending.get(&tid) {
                println!("    Thread {} waiting on uaddr=0x{:016x}", tid, uaddr);
            }
        }
    } else if let Some(blocked) = detect_total_blocking(&pending, &active_tids) {
        println!(
            "\n[lock-hunt] ⚠  DEADLOCK DETECTED — Tier 3: total blocking \
             (all active threads pending)"
        );
        println!("  Blocked threads: {:?}", blocked);
    } else {
        println!("[lock-hunt] No deadlock detected.");
        if pending.is_empty() {
            println!("  (모든 acquire가 완료됨 — 정상)");
        } else {
            println!("  (pending 요청은 있으나 deadlock 패턴 없음 — partial blocking)");
        }
    }
}
