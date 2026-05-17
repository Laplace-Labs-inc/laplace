//! Inbound QUIC accept loop for `MeshAgent`.
//!
//! Binds a QUIC server endpoint, accepts connections, and reads batched
//! Phase 3 context-aware, compression-aware frames from unidirectional
//! streams, forwarding each decoded message through an `mpsc` channel.
//!
//! ## Phase 3 wire format per frame
//!
//! ```text
//! [4 bytes BE u32: total_frame_len]
//! [1 byte: flags]
//!     bit 0 (0x01) — Layer 1: static dict active (payload starts with VarInt route_id)
//!     bit 1 (0x02) — Layer 2: dynamic dict active (tokens substituted)
//!     bit 2 (0x04) — Layer 3: LZ4 compressed payload
//!     bit 4 (0x10) — LaplaceContext present
//! [41 bytes: LaplaceContext LE]               ← only when bit 4 set
//! [N bytes: payload]                          ← LZ4-decompressed when bit 2 set
//! ```

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::domain::context::{LaplaceContext, CONTEXT_BYTES, CTX_FLAG};
use crate::domain::wire::compression::lz4_decompress;
use laplace_interfaces::domain::transport::pluggable::{
    BoundSocket, PacketInterceptor, SocketProvider,
};
use laplace_interfaces::{ProbeConfig, TransportPacket};

use super::outbound::FLAG_LAYER3;
use super::MeshAgentError;

/// Inbound message: flags byte + optional context header + payload bytes.
///
/// The flags byte tells the receiver which compression layers are active:
/// - `FLAG_LAYER1` (0x01): payload starts with a VarInt route_id
/// - `FLAG_LAYER2` (0x02): payload tokens use dynamic dict IDs
/// - `FLAG_LAYER3` (0x04): payload was LZ4 compressed (already decompressed here)
/// - `CTX_FLAG`   (0x10): context is `Some(_)`
pub type InboundMsg = (Option<LaplaceContext>, Vec<u8>, u8);

// ── InboundLoop ───────────────────────────────────────────────────────────────

/// Handle to the running inbound accept loop.
pub struct InboundLoop {
    _accept_task: tokio::task::JoinHandle<()>,
}

impl InboundLoop {
    /// Bind a QUIC server endpoint at `bind_addr` and start accepting connections.
    ///
    /// Uses `ProbeConfig::default()` for `max_frame_len`.
    pub async fn start(
        bind_addr: SocketAddr,
        tx: mpsc::UnboundedSender<InboundMsg>,
        socket_provider: Arc<dyn SocketProvider>,
        interceptor: Arc<dyn PacketInterceptor>,
    ) -> Result<Self, MeshAgentError> {
        Self::start_with_config(
            bind_addr,
            tx,
            &ProbeConfig::default(),
            socket_provider,
            interceptor,
        )
        .await
    }

    /// Bind a QUIC server endpoint with injected `ProbeConfig`, `SocketProvider`, and `PacketInterceptor`.
    pub async fn start_with_config(
        bind_addr: SocketAddr,
        tx: mpsc::UnboundedSender<InboundMsg>,
        cfg: &ProbeConfig,
        socket_provider: Arc<dyn SocketProvider>,
        interceptor: Arc<dyn PacketInterceptor>,
    ) -> Result<Self, MeshAgentError> {
        let max_frame_len = cfg.max_frame_len;
        let server_config = make_server_config()?;

        let bound = socket_provider
            .bind(bind_addr)
            .map_err(|e| MeshAgentError::Io(format!("bind {bind_addr}: {e}")))?;

        let udp_socket = match bound {
            BoundSocket::Os(s) => {
                s.set_nonblocking(true)
                    .map_err(|e| MeshAgentError::Io(format!("set_nonblocking: {e}")))?;
                s
            }
            BoundSocket::Virtual { .. } => {
                // Phase 4: virtual socket support pending
                return Err(MeshAgentError::Config(
                    "virtual socket not yet supported for inbound".into(),
                ));
            }
        };

        let endpoint = quinn::Endpoint::new(
            Default::default(),
            Some(server_config),
            udp_socket,
            Arc::new(quinn::TokioRuntime),
        )
        .map_err(|e| MeshAgentError::Quic(format!("endpoint new {bind_addr}: {e}")))?;

        tracing::info!(addr = %bind_addr, "MeshAgent inbound endpoint bound");

        let task = tokio::spawn(accept_loop(endpoint, tx, max_frame_len, interceptor));
        Ok(Self { _accept_task: task })
    }
}

// ── Accept loop ───────────────────────────────────────────────────────────────

async fn accept_loop(
    endpoint: quinn::Endpoint,
    tx: mpsc::UnboundedSender<InboundMsg>,
    max_frame_len: u32,
    interceptor: Arc<dyn PacketInterceptor>,
) {
    loop {
        match endpoint.accept().await {
            Some(incoming) => {
                let tx = tx.clone();
                let interceptor = interceptor.clone();
                tokio::spawn(async move {
                    match incoming.await {
                        Ok(conn) => {
                            tracing::debug!(
                                peer = %conn.remote_address(),
                                "MeshAgent: inbound connection"
                            );
                            handle_connection(conn, tx, max_frame_len, interceptor).await;
                        }
                        Err(e) => {
                            tracing::warn!(error = ?e, "MeshAgent: connection handshake failed");
                        }
                    }
                });
            }
            None => {
                tracing::info!("MeshAgent: inbound endpoint closed");
                break;
            }
        }
    }
}

async fn handle_connection(
    conn: quinn::Connection,
    tx: mpsc::UnboundedSender<InboundMsg>,
    max_frame_len: u32,
    interceptor: Arc<dyn PacketInterceptor>,
) {
    loop {
        match conn.accept_uni().await {
            Ok(recv_stream) => {
                let tx = tx.clone();
                let interceptor = interceptor.clone();
                tokio::spawn(async move {
                    read_batch_stream(recv_stream, tx, max_frame_len, interceptor).await;
                });
            }
            Err(quinn::ConnectionError::ApplicationClosed(_))
            | Err(quinn::ConnectionError::ConnectionClosed(_)) => {
                tracing::debug!("MeshAgent: inbound connection closed");
                break;
            }
            Err(e) => {
                tracing::warn!(error = ?e, "MeshAgent: accept_uni error");
                break;
            }
        }
    }
}

// ── Phase 3 frame parser ──────────────────────────────────────────────────────

/// Read a Phase 3 framed stream, parse all layers, and forward each message.
async fn read_batch_stream(
    mut stream: quinn::RecvStream,
    tx: mpsc::UnboundedSender<InboundMsg>,
    max_frame_len: u32,
    interceptor: Arc<dyn PacketInterceptor>,
) {
    loop {
        // ── 4-byte total_frame_len ────────────────────────────────────────────
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf).await {
            Ok(()) => {}
            Err(quinn::ReadExactError::FinishedEarly(_)) => break,
            Err(e) => {
                tracing::warn!(error = ?e, "MeshAgent: frame length read error");
                break;
            }
        }

        let total_frame_len = u32::from_be_bytes(len_buf);
        if total_frame_len == 0 {
            continue;
        }
        if total_frame_len > max_frame_len {
            tracing::warn!(
                total_frame_len,
                "MeshAgent: frame too large — dropping stream"
            );
            break;
        }

        // ── 1-byte flags ──────────────────────────────────────────────────────
        let mut flags_buf = [0u8; 1];
        if stream.read_exact(&mut flags_buf).await.is_err() {
            tracing::warn!("MeshAgent: flags read error");
            break;
        }
        let flags = flags_buf[0];
        let has_context = (flags & CTX_FLAG) != 0;

        // ── Optionally read LaplaceContext (41 bytes, bit 4) ──────────────────
        let ctx: Option<LaplaceContext> = if has_context {
            let mut ctx_buf = [0u8; CONTEXT_BYTES];
            if stream.read_exact(&mut ctx_buf).await.is_err() {
                tracing::warn!("MeshAgent: context read error");
                break;
            }
            Some(LaplaceContext::from_bytes(&ctx_buf))
        } else {
            None
        };

        // ── Payload (remaining bytes after flags + optional context) ──────────
        let ctx_len = if has_context { CONTEXT_BYTES as u32 } else { 0 };
        let payload_len = total_frame_len.saturating_sub(1 + ctx_len) as usize;

        let raw_payload = if payload_len > 0 {
            let mut buf = vec![0u8; payload_len];
            match stream.read_exact(&mut buf).await {
                Ok(()) => buf,
                Err(e) => {
                    tracing::warn!(error = ?e, "MeshAgent: payload read error");
                    break;
                }
            }
        } else {
            Vec::new()
        };

        // ── Layer 3: LZ4 decompression (bit 2) ────────────────────────────────
        let payload = if (flags & FLAG_LAYER3) != 0 {
            match lz4_decompress(&raw_payload) {
                Ok(decompressed) => decompressed,
                Err(e) => {
                    tracing::warn!(error = ?e, "MeshAgent: LZ4 decompression failed — dropping frame");
                    continue;
                }
            }
        } else {
            raw_payload
        };

        // ChaosInterceptor hook: on_receive may drop the packet deterministically (LEP-0015)
        let mut pkt_buf = TransportPacket::new(payload.clone(), 0);
        if interceptor.on_receive(&mut pkt_buf).is_err() {
            // Chaos: packet dropped — no retry (LEP-0015 Ghost Constraint)
            continue;
        }

        // Forward (context, payload, flags) — Layer 1/2 decoding is the caller's responsibility
        if tx.send((ctx, payload, flags)).is_err() {
            break;
        }
    }
}

// ── TLS helpers ───────────────────────────────────────────────────────────────

pub fn make_server_config() -> Result<quinn::ServerConfig, MeshAgentError> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()])
        .map_err(|e| MeshAgentError::Config(format!("cert gen: {e}")))?;

    let key_bytes = cert.signing_key.serialize_der();
    let cert_der = cert.cert.der().to_vec();

    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let key = PrivateKeyDer::Pkcs8(key_bytes.into());
    let cert_der = CertificateDer::from(cert_der);

    let mut config = quinn::ServerConfig::with_single_cert(vec![cert_der], key)
        .map_err(|e| MeshAgentError::Config(format!("server config: {e}")))?;

    let mut transport = quinn::TransportConfig::default();
    transport.max_concurrent_uni_streams(256u32.into());
    config.transport_config(Arc::new(transport));

    Ok(config)
}
