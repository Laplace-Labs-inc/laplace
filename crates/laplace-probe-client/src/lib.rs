//! laplace-probe-client — probe-edge WebSocket 클라이언트.
//!
//! [Ghost Constraint]: emit()은 절대 block 금지. try_send() 실패 시 조용히 드롭.

use std::time::Duration;

use anyhow::Result;
use futures_util::SinkExt;
use tokio::sync::mpsc;
use tokio::time;
use tokio_tungstenite::{connect_async, tungstenite::Message};

pub use laplace_probe_common::RawProbeEvent;

// ── Config ────────────────────────────────────────────────────────────────────

/// probe-edge 연결 설정.
#[derive(Debug, Clone)]
pub struct ProbeClientConfig {
    /// WebSocket URL (e.g. "ws://localhost:8443/ws")
    pub edge_url: String,
    /// JWT (현재 probe-edge는 미검증 — 향후 인증 확장용)
    pub jwt: String,
    /// 배치 flush 최대 이벤트 수. Default: 32
    pub max_batch_size: usize,
    /// 배치 flush 간격 (ms). Default: 100
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

/// 비동기 WebSocket 클라이언트 핸들.
///
/// `connect()` 가 백그라운드 태스크를 spawn하고 이 핸들을 반환한다.
/// Clone은 채널 복사이므로 O(1).
#[derive(Clone)]
pub struct ProbeClient {
    tx: mpsc::Sender<RawProbeEvent>,
}

impl ProbeClient {
    /// probe-edge에 WebSocket 연결을 맺고 백그라운드 전송 태스크를 spawn한다.
    ///
    /// 호출 시점에 연결이 즉시 맺어지지 않아도 ok — 큐에 먼저 쌓고 연결 후 전송.
    /// [Ghost Constraint]: tokio 런타임 내에서만 호출 가능.
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

    /// RawProbeEvent를 큐에 넣는다. 논블로킹.
    ///
    /// [Ghost Constraint]: 큐가 꽉 차면 조용히 드롭 — 관측자 효과 방지.
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
