// SPDX-License-Identifier: Apache-2.0
//! Local deterministic Probe Common microbenchmarks.

use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use laplace_probe_common::{DecodedProbeEvent, ProbeEventDecoder, ProbeEventType, RawProbeEvent};

const SEED: u64 = 42;
const RUNS: usize = 100;

fn make_raw_event(seed: u64, event_type: ProbeEventType, idx: usize) -> RawProbeEvent {
    RawProbeEvent {
        timestamp_ns: 1_700_000_000_000_000_000 + idx as u64,
        tid: (1_000 + idx) as u32,
        pid: 900,
        event_type: event_type as u8,
        l4_proto: 17,
        status_code: 200,
        _pad0: 0,
        resource_id: seed ^ (idx as u64).wrapping_mul(131),
        peer_addr: 0x7f00_0001,
        peer_port: 443,
        local_port: 10_000 + idx as u32,
        payload_hash: seed.rotate_left((idx % 31) as u32),
        payload_len: 512 + idx as u32,
        operation_hash: 0xfeed_0000 ^ idx as u32,
        latency_ns: 25_000 + idx as u64,
        _pad1: 0,
        correlation_id: seed ^ idx as u64,
        cpu_id: (idx % 4) as u32,
        depth: (idx % 16) as u32,
        comm: command_name(idx),
        parent_tid: (2_000 + idx) as u64,
        _reserved: 0,
    }
}

fn command_name(idx: usize) -> [u8; 16] {
    let label = format!("probe-{idx:04}");
    let mut out = [0u8; 16];
    out[..label.len()].copy_from_slice(label.as_bytes());
    out
}

fn make_events(count: usize, event_type: ProbeEventType, seed: u64) -> Vec<RawProbeEvent> {
    (0..count)
        .map(|idx| make_raw_event(seed, event_type, idx))
        .collect()
}

fn encode_raw_event(event: &RawProbeEvent) -> [u8; 128] {
    let mut out = [0u8; 128];
    out[0..8].copy_from_slice(&event.timestamp_ns.to_le_bytes());
    out[8..12].copy_from_slice(&event.tid.to_le_bytes());
    out[12..16].copy_from_slice(&event.pid.to_le_bytes());
    out[16] = event.event_type;
    out[17] = event.l4_proto;
    out[18..20].copy_from_slice(&event.status_code.to_le_bytes());
    out[24..32].copy_from_slice(&event.resource_id.to_le_bytes());
    out[32..40].copy_from_slice(&event.peer_addr.to_le_bytes());
    out[40..44].copy_from_slice(&event.peer_port.to_le_bytes());
    out[44..48].copy_from_slice(&event.local_port.to_le_bytes());
    out[48..56].copy_from_slice(&event.payload_hash.to_le_bytes());
    out[56..60].copy_from_slice(&event.payload_len.to_le_bytes());
    out[60..64].copy_from_slice(&event.operation_hash.to_le_bytes());
    out[64..72].copy_from_slice(&event.latency_ns.to_le_bytes());
    out[80..88].copy_from_slice(&event.correlation_id.to_le_bytes());
    out[88..92].copy_from_slice(&event.cpu_id.to_le_bytes());
    out[92..96].copy_from_slice(&event.depth.to_le_bytes());
    out[96..112].copy_from_slice(&event.comm);
    out[112..120].copy_from_slice(&event.parent_tid.to_le_bytes());
    out
}

fn decode_wire_event(frame: &[u8; 128]) -> RawProbeEvent {
    let mut comm = [0u8; 16];
    comm.copy_from_slice(&frame[96..112]);
    RawProbeEvent {
        timestamp_ns: u64::from_le_bytes(frame[0..8].try_into().expect("timestamp")),
        tid: u32::from_le_bytes(frame[8..12].try_into().expect("tid")),
        pid: u32::from_le_bytes(frame[12..16].try_into().expect("pid")),
        event_type: frame[16],
        l4_proto: frame[17],
        status_code: u16::from_le_bytes(frame[18..20].try_into().expect("status")),
        _pad0: 0,
        resource_id: u64::from_le_bytes(frame[24..32].try_into().expect("resource")),
        peer_addr: u64::from_le_bytes(frame[32..40].try_into().expect("peer addr")),
        peer_port: u32::from_le_bytes(frame[40..44].try_into().expect("peer port")),
        local_port: u32::from_le_bytes(frame[44..48].try_into().expect("local port")),
        payload_hash: u64::from_le_bytes(frame[48..56].try_into().expect("payload hash")),
        payload_len: u32::from_le_bytes(frame[56..60].try_into().expect("payload len")),
        operation_hash: u32::from_le_bytes(frame[60..64].try_into().expect("op hash")),
        latency_ns: u64::from_le_bytes(frame[64..72].try_into().expect("latency")),
        _pad1: 0,
        correlation_id: u64::from_le_bytes(frame[80..88].try_into().expect("correlation")),
        cpu_id: u32::from_le_bytes(frame[88..92].try_into().expect("cpu")),
        depth: u32::from_le_bytes(frame[92..96].try_into().expect("depth")),
        comm,
        parent_tid: u64::from_le_bytes(frame[112..120].try_into().expect("parent")),
        _reserved: 0,
    }
}

fn fold_wire_frame(acc: u64, frame: &[u8; 128]) -> u64 {
    acc.wrapping_add(u64::from(frame[16]))
        .wrapping_add(u64::from(frame[18]) << 8)
        .wrapping_add(u64::from(frame[56]) << 16)
        .wrapping_add(u64::from(frame[60]) << 24)
        .wrapping_add(u64::from(frame[96]) << 32)
        .wrapping_add(u64::from(frame[112]) << 40)
}

fn fold_decoded_event(acc: u64, event: DecodedProbeEvent) -> u64 {
    match event {
        DecodedProbeEvent::NetworkRequest {
            tid,
            timestamp_ns,
            resource_id,
            operation_hash,
            payload_hash,
            payload_len,
            status_code,
            latency_ns,
            peer_addr,
            peer_port,
        } => acc
            .wrapping_add(u64::from(tid))
            .wrapping_add(timestamp_ns)
            .wrapping_add(resource_id)
            .wrapping_add(u64::from(operation_hash))
            .wrapping_add(payload_hash)
            .wrapping_add(u64::from(payload_len))
            .wrapping_add(u64::from(status_code))
            .wrapping_add(latency_ns)
            .wrapping_add(peer_addr)
            .wrapping_add(u64::from(peer_port)),
        DecodedProbeEvent::NetworkResponse {
            tid,
            timestamp_ns,
            resource_id,
            operation_hash,
            payload_hash,
            payload_len,
            status_code,
            latency_ns,
        } => acc
            .wrapping_add(u64::from(tid))
            .wrapping_add(timestamp_ns)
            .wrapping_add(resource_id)
            .wrapping_add(u64::from(operation_hash))
            .wrapping_add(payload_hash)
            .wrapping_add(u64::from(payload_len))
            .wrapping_add(u64::from(status_code))
            .wrapping_add(latency_ns),
        DecodedProbeEvent::LockAcquire {
            tid,
            timestamp_ns,
            mutex_addr,
            contention_ns,
        }
        | DecodedProbeEvent::LockAcquired {
            tid,
            timestamp_ns,
            mutex_addr,
            contention_ns,
        } => acc
            .wrapping_add(u64::from(tid))
            .wrapping_add(timestamp_ns)
            .wrapping_add(mutex_addr)
            .wrapping_add(contention_ns),
        DecodedProbeEvent::LockRelease {
            tid,
            timestamp_ns,
            mutex_addr,
        }
        | DecodedProbeEvent::LockContention {
            tid,
            timestamp_ns,
            mutex_addr,
        } => acc
            .wrapping_add(u64::from(tid))
            .wrapping_add(timestamp_ns)
            .wrapping_add(mutex_addr),
        DecodedProbeEvent::SchedSwitch {
            prev_tid,
            next_tid,
            timestamp_ns,
            cpu_id,
        } => acc
            .wrapping_add(u64::from(prev_tid))
            .wrapping_add(u64::from(next_tid))
            .wrapping_add(timestamp_ns)
            .wrapping_add(u64::from(cpu_id)),
        DecodedProbeEvent::ThreadSpawn {
            parent_tid,
            child_tid,
            timestamp_ns,
        } => acc
            .wrapping_add(u64::from(parent_tid))
            .wrapping_add(u64::from(child_tid))
            .wrapping_add(timestamp_ns),
        DecodedProbeEvent::ThreadExit { tid, timestamp_ns } => {
            acc.wrapping_add(u64::from(tid)).wrapping_add(timestamp_ns)
        }
        DecodedProbeEvent::ConnOpen {
            tid,
            timestamp_ns,
            resource_id,
            peer_addr,
            peer_port,
        } => acc
            .wrapping_add(u64::from(tid))
            .wrapping_add(timestamp_ns)
            .wrapping_add(resource_id)
            .wrapping_add(peer_addr)
            .wrapping_add(u64::from(peer_port)),
        DecodedProbeEvent::ConnClose {
            tid,
            timestamp_ns,
            resource_id,
        } => acc
            .wrapping_add(u64::from(tid))
            .wrapping_add(timestamp_ns)
            .wrapping_add(resource_id),
        DecodedProbeEvent::RwLockReadAcquire {
            tid,
            timestamp_ns,
            rwlock_addr,
        }
        | DecodedProbeEvent::RwLockReadRelease {
            tid,
            timestamp_ns,
            rwlock_addr,
        }
        | DecodedProbeEvent::RwLockWriteAcquire {
            tid,
            timestamp_ns,
            rwlock_addr,
        }
        | DecodedProbeEvent::RwLockWriteRelease {
            tid,
            timestamp_ns,
            rwlock_addr,
        } => acc
            .wrapping_add(u64::from(tid))
            .wrapping_add(timestamp_ns)
            .wrapping_add(rwlock_addr),
        DecodedProbeEvent::AtomicLoad {
            tid,
            timestamp_ns,
            addr,
        }
        | DecodedProbeEvent::AtomicStore {
            tid,
            timestamp_ns,
            addr,
        }
        | DecodedProbeEvent::AtomicRmw {
            tid,
            timestamp_ns,
            addr,
        } => acc
            .wrapping_add(u64::from(tid))
            .wrapping_add(timestamp_ns)
            .wrapping_add(addr),
        DecodedProbeEvent::SemaphoreAcquire {
            tid,
            timestamp_ns,
            sem_addr,
        }
        | DecodedProbeEvent::SemaphoreRelease {
            tid,
            timestamp_ns,
            sem_addr,
        } => acc
            .wrapping_add(u64::from(tid))
            .wrapping_add(timestamp_ns)
            .wrapping_add(sem_addr),
        DecodedProbeEvent::ChannelSend {
            tid,
            timestamp_ns,
            channel_addr,
        }
        | DecodedProbeEvent::ChannelRecv {
            tid,
            timestamp_ns,
            channel_addr,
        } => acc
            .wrapping_add(u64::from(tid))
            .wrapping_add(timestamp_ns)
            .wrapping_add(channel_addr),
    }
}

fn probe_common_replay_digest(seed: u64) -> u64 {
    let decoder = ProbeEventDecoder::new();
    [
        ProbeEventType::NetRequest,
        ProbeEventType::LockAcquire,
        ProbeEventType::SchedSwitch,
        ProbeEventType::ChannelSend,
    ]
    .into_iter()
    .flat_map(|event_type| make_events(32, event_type, seed))
    .map(|event| encode_raw_event(&event))
    .map(|frame| decode_wire_event(&frame))
    .filter_map(|event| decoder.decode(&event))
    .fold(seed, fold_decoded_event)
}

fn bench_common_protocol_encode_decode_ns(c: &mut Criterion) {
    let decoder = ProbeEventDecoder::new();
    let events = make_events(1_024, ProbeEventType::NetRequest, SEED);
    let frames: Vec<_> = events.iter().map(encode_raw_event).collect();
    let mut group = c.benchmark_group("probe_common_protocol_encode_decode_ns");
    group.throughput(Throughput::Elements(events.len() as u64));
    group.bench_function("encode_raw_event", |b| {
        b.iter(|| {
            let checksum = events
                .iter()
                .map(|event| encode_raw_event(black_box(event)))
                .fold(0u64, |acc, frame| fold_wire_frame(acc, &frame));
            black_box(checksum)
        })
    });
    group.bench_function("decode_raw_event", |b| {
        b.iter(|| {
            let checksum = frames
                .iter()
                .map(|frame| decode_wire_event(black_box(frame)))
                .filter_map(|event| decoder.decode(black_box(&event)))
                .fold(0u64, fold_decoded_event);
            black_box(checksum)
        })
    });
    group.finish();
}

fn bench_common_wire_format_size_per_message_type(c: &mut Criterion) {
    let message_types = [
        ("net_request", ProbeEventType::NetRequest),
        ("lock_acquire", ProbeEventType::LockAcquire),
        ("sched_switch", ProbeEventType::SchedSwitch),
        ("channel_send", ProbeEventType::ChannelSend),
    ];
    let mut group = c.benchmark_group("probe_common_wire_format_size_per_message_type");
    for (name, event_type) in message_types {
        let events = make_events(512, event_type, SEED);
        group.throughput(Throughput::Bytes((events.len() * 128) as u64));
        group.bench_with_input(
            BenchmarkId::new("message_type", name),
            &events,
            |b, sample| {
                b.iter(|| {
                    let checksum = black_box(sample)
                        .iter()
                        .map(|event| encode_raw_event(black_box(event)))
                        .fold(0u64, |acc, frame| fold_wire_frame(acc, &frame));
                    black_box(checksum)
                })
            },
        );
    }
    group.finish();
}

fn bench_probe_common_determinism_replay(c: &mut Criterion) {
    c.bench_function("probe_common_determinism_replay_3runs", |b| {
        b.iter(|| {
            let r1 = probe_common_replay_digest(black_box(SEED));
            let r2 = probe_common_replay_digest(black_box(SEED));
            let r3 = probe_common_replay_digest(black_box(SEED));
            assert_eq!(r1, r2);
            assert_eq!(r2, r3);
            black_box((r1, r2, r3))
        })
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(RUNS)
        .warm_up_time(Duration::from_secs(5))
        .measurement_time(Duration::from_secs(10))
        .confidence_level(0.95)
        .significance_level(0.05);
    targets =
        bench_common_protocol_encode_decode_ns,
        bench_common_wire_format_size_per_message_type,
        bench_probe_common_determinism_replay
}
criterion_main!(benches);
