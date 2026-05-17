//! Probe Listener — KNUL (QUIC) 수신 서버
//!
//! `start_listener(bind_addr)` 를 호출하면 해당 주소에서 QUIC 서버를 기동하고,
//! `laplace-probe`(`MeshClient`)가 보내는 길이-prefix 프레임을 수신해
//! `ProbeEvent`로 역직렬화한 뒤 채널로 전달한다.
//!
//! **Wire format (MeshClient와 동일):**
//! ```text
//! [4 bytes BE u32: payload_len][payload_len bytes: JSON(ProbeEvent)]
//! ```

#![cfg(all(feature = "twin", feature = "verification"))]

use std::sync::Arc;

use anyhow::anyhow;
use laplace_probe::domain::transport::{KnulConnection, KnulStream};
use laplace_probe::infrastructure::transport::quinn_impl::QuinnConnection;
use laplace_probe::ProbeEvent;
use tokio::sync::mpsc;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// QUIC 서버를 `bind_addr`에 기동하고, 수신된 [`ProbeEvent`]를 흘려보내는
/// 채널 수신단을 반환한다.
///
/// 호출 후 즉시 백그라운드 태스크가 스폰되므로 반환된 `Receiver`만 보관하면 된다.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "30_Axiom_Panopticon",
        link = "LEP-0012-laplace-axiom-infrastructure_panopticon"
    )
)]
pub async fn start_listener(
    bind_addr: &str,
) -> anyhow::Result<mpsc::UnboundedReceiver<ProbeEvent>> {
    let (tx, rx) = mpsc::unbounded_channel::<ProbeEvent>();

    let bind_addr_parsed: std::net::SocketAddr = bind_addr
        .parse()
        .map_err(|e| anyhow!("addr parse {bind_addr}: {e}"))?;

    let server_cfg =
        laplace_probe::make_server_config().map_err(|e| anyhow!("server config: {e}"))?;

    let udp = std::net::UdpSocket::bind(bind_addr_parsed)
        .map_err(|e| anyhow!("udp bind {bind_addr}: {e}"))?;
    udp.set_nonblocking(true)
        .map_err(|e| anyhow!("set_nonblocking: {e}"))?;
    let endpoint = quinn::Endpoint::new(
        Default::default(),
        Some(server_cfg),
        udp,
        Arc::new(quinn::TokioRuntime),
    )
    .map_err(|e| anyhow!("endpoint new {bind_addr}: {e}"))?;

    let bound = endpoint
        .local_addr()
        .map_err(|e| anyhow!("local_addr: {e}"))?
        .to_string();

    tracing::info!(addr = %bound, "Axiom probe listener started (KNUL/QUIC)");

    tokio::spawn(async move {
        accept_loop(endpoint, tx).await;
    });

    Ok(rx)
}

// ── Accept loop ───────────────────────────────────────────────────────────────

async fn accept_loop(endpoint: quinn::Endpoint, tx: mpsc::UnboundedSender<ProbeEvent>) {
    loop {
        match endpoint.accept().await {
            Some(incoming) => {
                let tx = tx.clone();
                tokio::spawn(async move {
                    match incoming.await {
                        Ok(conn) => {
                            let boxed: Box<dyn KnulConnection> =
                                Box::new(QuinnConnection::new(conn));
                            accept_streams(boxed, tx).await;
                        }
                        Err(e) => {
                            tracing::warn!(error = ?e, "Axiom probe listener: handshake failed");
                        }
                    }
                });
            }
            None => {
                tracing::info!("Axiom probe listener: endpoint shut down");
                break;
            }
        }
    }
}

// ── Per-connection stream acceptor ────────────────────────────────────────────

async fn accept_streams(mut conn: Box<dyn KnulConnection>, tx: mpsc::UnboundedSender<ProbeEvent>) {
    loop {
        match conn.accept_stream().await {
            Ok(stream) => {
                let tx = tx.clone();
                tokio::spawn(async move {
                    read_one_event(stream, tx).await;
                });
            }
            Err(_) => break,
        }
    }
}

// ── Single-stream frame reader ────────────────────────────────────────────────

#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "30_Axiom_Panopticon",
        link = "LEP-0012-laplace-axiom-infrastructure_panopticon"
    )
)]
async fn read_one_event(mut stream: Box<dyn KnulStream>, tx: mpsc::UnboundedSender<ProbeEvent>) {
    // 1. Read 4-byte big-endian length prefix.
    let mut len_buf = [0u8; 4];
    match stream.read(&mut len_buf).await {
        Ok(n) if n == 4 => {}
        Ok(_) => {
            tracing::debug!("probe_listener: short read on length prefix — ignored");
            return;
        }
        Err(e) => {
            tracing::debug!(error = ?e, "probe_listener: length read error");
            return;
        }
    }

    let payload_len = u32::from_be_bytes(len_buf) as usize;
    if payload_len == 0 || payload_len > 1024 * 1024 {
        tracing::warn!(
            payload_len,
            "probe_listener: implausible frame size — ignored"
        );
        return;
    }

    // 2. Read JSON payload.
    let mut payload = vec![0u8; payload_len];
    match stream.read(&mut payload).await {
        Ok(n) if n == payload_len => {}
        Ok(n) => {
            tracing::warn!(
                got = n,
                expected = payload_len,
                "probe_listener: short payload read"
            );
            return;
        }
        Err(e) => {
            tracing::debug!(error = ?e, "probe_listener: payload read error");
            return;
        }
    }

    // 3. Deserialize and forward.
    match serde_json::from_slice::<ProbeEvent>(&payload) {
        Ok(event) => {
            tracing::debug!(event = ?event, "probe_listener: ProbeEvent received");
            let _ = tx.send(event);
        }
        Err(e) => {
            tracing::warn!(error = %e, "probe_listener: deserialize failed");
        }
    }
}
