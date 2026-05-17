//! Phase 1 smoke-test: load eBPF bytecode, attach `sched:sched_switch`,
//! and print decoded `RawProbeEvent`s from the ring buffer.
//!
//! # Build & Run
//!
//! 1. Compile the eBPF program (see instructions at bottom of this file).
//! 2. Run as root (eBPF requires CAP_BPF / CAP_SYS_ADMIN):
//!    ```bash
//!    sudo cargo run --bin probe-test
//!    ```

use aya::{maps::RingBuf, programs::TracePoint, Ebpf};
use aya_log::EbpfLogger;
use bytemuck::from_bytes;
use laplace_probe_common::{ProbeEventType, RawProbeEvent};
use tokio::io::unix::AsyncFd;

/// Path to the compiled eBPF object file.
///
/// Compile with:
///   cd crates/probe/laplace-probe-ebpf
///   cargo +nightly build --release
/// Output: target/bpfel-unknown-none/release/laplace-probe-ebpf
const EBPF_OBJECT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../laplace-probe-ebpf/target/bpfel-unknown-none/release/laplace-probe-ebpf"
));

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── 1. Load eBPF bytecode ─────────────────────────────────────────────────
    let mut bpf = Ebpf::load(EBPF_OBJECT)?;

    if let Err(e) = EbpfLogger::init(&mut bpf) {
        eprintln!("eBPF logger init warning (non-fatal): {e}");
    }

    // ── 2. Attach sched_switch tracepoint ─────────────────────────────────────
    let program: &mut TracePoint = bpf.program_mut("sched_switch").unwrap().try_into()?;
    program.load()?;
    program.attach("sched", "sched_switch")?;
    println!("[probe-test] Attached sched:sched_switch tracepoint. Listening for events…");

    // ── 3. Set up async RingBuf consumer ─────────────────────────────────────
    let ring_buf = RingBuf::try_from(bpf.map_mut("EVENTS").unwrap())?;
    let mut async_fd = AsyncFd::new(ring_buf)?;

    // ── 4. Event loop ─────────────────────────────────────────────────────────
    loop {
        let mut guard = async_fd.readable_mut().await?;
        let ring = guard.get_inner_mut();

        while let Some(item) = ring.next() {
            let data: &[u8] = &item;
            if data.len() < std::mem::size_of::<RawProbeEvent>() {
                eprintln!("short read: {} bytes", data.len());
                continue;
            }
            let event: &RawProbeEvent = from_bytes(&data[..std::mem::size_of::<RawProbeEvent>()]);

            if event.event_type == ProbeEventType::SchedSwitch as u8 {
                let comm = std::str::from_utf8(&event.comm)
                    .unwrap_or("?")
                    .trim_end_matches('\0');
                println!(
                    "[SchedSwitch] ts={} cpu={} prev_pid={} next_pid={} next_comm={}",
                    event.timestamp_ns,
                    event.cpu_id,
                    event.resource_id, // prev_pid stored here
                    event.peer_addr,   // next_pid stored here
                    comm,
                );
            }
        }

        guard.clear_ready();
    }
}
