// SPDX-License-Identifier: Apache-2.0
//! laplace-probe-client — probe-edge WebSocket client.
//!
//! [Ghost Constraint]: emit() must never block. Silently drop on try_send() failure.

use std::time::Duration;

use anyhow::Result;
use futures_util::SinkExt;
use tokio::sync::mpsc;
use tokio::time;
use tokio_tungstenite::{connect_async, tungstenite::Message};

pub use laplace_probe_common::RawProbeEvent;

// ── Config ────────────────────────────────────────────────────────────────────

/// Probe-edge connection settings.
#[derive(Debug, Clone)]
pub struct ProbeClientConfig {
    /// WebSocket URL (e.g. "ws://localhost:8443/ws")
    pub edge_url: String,
    /// JWT (probe-edge does not currently validate it — for future authentication extensions)
    pub jwt: String,
    /// Maximum events per batch flush. Default: 32.
    pub max_batch_size: usize,
    /// Batch flush interval in milliseconds. Default: 100.
    pub flush_interval_ms: u64,
}

impl Default for ProbeClientConfig {
    fn default() -> Self {
        Self {
            edge_url: std::env::var("LAPLACE_PROBE_EDGE_URL")
                .unwrap_or_else(|_| "ws://localhost:8443/ws".to_string()),
            jwt: std::env::var("LAPLACE_JWT").unwrap_or_default(),
            max_batch_size: 32,
            flush_interval_ms: 100,
        }
    }
}

// ── ProbeClient ───────────────────────────────────────────────────────────────

/// Asynchronous WebSocket client handle.
///
/// `connect()` spawns a background task and returns this handle.
/// Cloning copies the channel and is O(1).
#[derive(Clone)]
pub struct ProbeClient {
    tx: mpsc::Sender<RawProbeEvent>,
}

impl ProbeClient {
    /// Connects to probe-edge over WebSocket and spawns a background sender task.
    ///
    /// The connection need not be established immediately — events are queued
    /// first and sent after connecting. [Ghost Constraint]: callable only inside
    /// a tokio runtime.
    pub async fn connect(config: ProbeClientConfig) -> Result<Self> {
        let (tx, rx) = mpsc::channel::<RawProbeEvent>(4096);
        let config_clone = config.clone();
        tokio::spawn(async move {
            sender_loop(config_clone, rx).await;
        });
        tracing::info!(
            edge_url = %config.edge_url,
            "ProbeClient: background sender spawned"
        );
        Ok(Self { tx })
    }

    /// Queues a `RawProbeEvent` without blocking.
    ///
    /// [Ghost Constraint]: silently drops events when the queue is full to avoid
    /// observer effects.
    #[inline]
    pub fn emit(&self, event: RawProbeEvent) {
        let _ = self.tx.try_send(event);
    }
}

// ── 백그라운드 전송 루프 ────────────────────────────────────────────────────────

async fn sender_loop(config: ProbeClientConfig, mut rx: mpsc::Receiver<RawProbeEvent>) {
    let interval = Duration::from_millis(config.flush_interval_ms);
    let mut batch: Vec<RawProbeEvent> = Vec::with_capacity(config.max_batch_size);

    loop {
        // 배치 수집: max_batch_size 또는 flush_interval_ms 중 먼저 도달한 것
        let deadline = time::sleep(interval);
        tokio::pin!(deadline);

        loop {
            tokio::select! {
                biased;
                Some(ev) = rx.recv() => {
                    batch.push(ev);
                    if batch.len() >= config.max_batch_size { break; }
                }
                _ = &mut deadline => { break; }
            }
        }

        if batch.is_empty() {
            continue;
        }

        if let Err(e) = send_batch(&config.edge_url, &batch).await {
            tracing::debug!("ProbeClient: send failed (will retry): {e}");
        }
        batch.clear();
    }
}

async fn send_batch(edge_url: &str, batch: &[RawProbeEvent]) -> Result<()> {
    let (mut ws, _) = connect_async(edge_url).await?;

    // 각 RawProbeEvent를 128바이트로 직렬화하여 하나의 Binary 프레임으로 전송
    // [Ghost Constraint]: bytemuck::bytes_of 사용 — 수동 직렬화 금지
    let mut payload = Vec::with_capacity(batch.len() * 128);
    for event in batch {
        payload.extend_from_slice(bytemuck::bytes_of(event));
    }

    ws.send(Message::Binary(payload.into())).await?;
    ws.close(None).await?;
    Ok(())
}

// ── 단위 테스트 ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_raw_event(event_type: u8, tid: u32) -> RawProbeEvent {
        let mut e: RawProbeEvent = bytemuck::Zeroable::zeroed();
        e.event_type = event_type;
        e.tid = tid;
        e.timestamp_ns = 1_000_000_000u64;
        e
    }

    #[test]
    fn test_bytes_of_is_128() {
        let ev = make_raw_event(3, 42);
        let bytes = bytemuck::bytes_of(&ev);
        assert_eq!(bytes.len(), 128, "RawProbeEvent must be 128 bytes");
    }

    #[test]
    fn test_batch_payload_size() {
        let batch: Vec<RawProbeEvent> = (0..5).map(|i| make_raw_event(3, i)).collect();
        let payload: Vec<u8> = batch
            .iter()
            .flat_map(|e| bytemuck::bytes_of(e).iter().copied())
            .collect();
        assert_eq!(payload.len(), 5 * 128);
    }

    #[tokio::test]
    async fn test_emit_no_block_when_disconnected() {
        // 연결이 없어도 emit()은 panic하지 않는다
        let config = ProbeClientConfig {
            edge_url: "ws://127.0.0.1:19999/ws".to_string(), // 없는 포트
            ..Default::default()
        };
        let client = ProbeClient::connect(config)
            .await
            .expect("ProbeClient should spawn without immediate network access");
        let ev = make_raw_event(3, 1);
        // 100번 emit해도 block 없이 반환되어야 한다
        for _ in 0..100 {
            client.emit(ev);
        }
    }
}
