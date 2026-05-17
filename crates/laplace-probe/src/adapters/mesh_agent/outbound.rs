// SPDX-License-Identifier: Apache-2.0
//! Outbound connection pool and batch flusher for `MeshAgent`.
//!
//! ## Batching strategy
//!
//! For each connected peer a dedicated background task accumulates outbound
//! messages and flushes them when **either** of the following thresholds is
//! reached:
//!
//! - **Count**: 64 messages in the buffer, OR
//! - **Time**: 100 ms have elapsed since the last flush
//!
//! ## Phase 3 wire format
//!
//! ```text
//! [4 bytes BE u32: total_frame_len]           ← len of (flags + ctx? + payload)
//! [1 byte: flags]
//!     bit 0 (0x01) — Layer 1: static dict active (payload starts with VarInt route_id)
//!     bit 1 (0x02) — Layer 2: dynamic dict active (tokens substituted)
//!     bit 2 (0x04) — Layer 3: LZ4 compressed payload (applied when > 4 KiB)
//!     bit 4 (0x10) — LaplaceContext present
//! [41 bytes: LaplaceContext LE]               ← only when bit 4 set
//! [N bytes: payload]                          ← possibly LZ4-compressed (bit 2)
//! ```
//!
//! LZ4 compression is applied automatically when bit 2 is requested **and**
//! the payload exceeds `LZ4_COMPRESSION_THRESHOLD` (4 KiB).

use dashmap::DashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::domain::context::{LaplaceContext, CONTEXT_BYTES, CTX_FLAG};
use crate::domain::wire::compression::{lz4_compress, LZ4_COMPRESSION_THRESHOLD};
use laplace_interfaces::domain::transport::pluggable::{
    BoundSocket, OsSocketProvider, SocketProvider,
};
use laplace_interfaces::ProbeConfig;

use super::MeshAgentError;

// ── Compression layer flags ───────────────────────────────────────────────────

/// bit 0: Layer 1 — static dictionary VarInt route ID active.
pub const FLAG_LAYER1: u8 = 0x01;
/// bit 1: Layer 2 — dynamic dictionary token substitution active.
pub const FLAG_LAYER2: u8 = 0x02;
/// bit 2: Layer 3 — LZ4 byte compression active.
pub const FLAG_LAYER3: u8 = 0x04;

/// A single message enqueued for batched delivery: context + payload.
pub(super) type OutboundMsg = (LaplaceContext, Vec<u8>);

// ── OutboundPool ─────────────────────────────────────────────────────────────

/// Pool of persistent outbound connections + per-peer batch flushers.
pub struct OutboundPool {
    /// Raw quinn connections, shared with the batch flusher tasks.
    connections: Arc<DashMap<SocketAddr, Arc<quinn::Connection>>>,
    /// Per-peer channel to enqueue (context, payload) for batched sending.
    senders: DashMap<SocketAddr, mpsc::UnboundedSender<OutboundMsg>>,
    /// Shared client quinn::Endpoint (one per OutboundPool).
    client_endpoint: tokio::sync::Mutex<Option<quinn::Endpoint>>,
    /// Active compression layer flags (shared with batch flusher tasks).
    active_flags: Arc<AtomicU8>,
    /// Maximum messages per batch (injected from config).
    batch_size: usize,
    /// Maximum idle time before flush (injected from config).
    flush_interval: Duration,
    /// Socket provider: OS bind in production, in-memory channel in Distributed Axiom.
    socket_provider: Arc<dyn SocketProvider>,
}

impl OutboundPool {
    /// Create a new pool with default `ProbeConfig` values.
    pub fn new() -> Self {
        Self::with_config(&ProbeConfig::default())
    }

    /// Create a new pool with injected `ProbeConfig`.
    pub fn with_config(cfg: &ProbeConfig) -> Self {
        Self {
            connections: Arc::new(DashMap::new()),
            senders: DashMap::new(),
            client_endpoint: tokio::sync::Mutex::new(None),
            active_flags: Arc::new(AtomicU8::new(0)),
            batch_size: cfg.batch_size as usize,
            flush_interval: Duration::from_millis(cfg.flush_interval_ms),
            socket_provider: Arc::new(OsSocketProvider),
        }
    }

    /// Set the active compression layer flags for all outbound frames.
    ///
    /// `flags` is a bitmask of `FLAG_LAYER1 | FLAG_LAYER2 | FLAG_LAYER3`.
    /// The `CTX_FLAG` (bit 4) is always set automatically; callers need not
    /// include it here.
    pub fn set_compression_flags(&self, flags: u8) {
        self.active_flags.store(flags, Ordering::Relaxed);
    }

    /// Read the current compression flags.
    pub fn compression_flags(&self) -> u8 {
        self.active_flags.load(Ordering::Relaxed)
    }

    /// Open or reuse a persistent outbound QUIC connection to `addr`.
    pub async fn connect(&self, addr: SocketAddr) -> Result<(), MeshAgentError> {
        if self.connections.contains_key(&addr) {
            return Ok(());
        }

        let conn = self.open_connection(addr).await?;
        let conn = Arc::new(conn);
        self.connections.insert(addr, conn.clone());

        let (tx, rx) = mpsc::unbounded_channel::<OutboundMsg>();
        self.senders.insert(addr, tx);

        let pool_conns = self.connections.clone();
        let shared_flags = self.active_flags.clone();
        let batch_size = self.batch_size;
        let flush_interval = self.flush_interval;
        tokio::spawn(run_batch_flusher(
            addr,
            conn,
            pool_conns,
            rx,
            shared_flags,
            batch_size,
            flush_interval,
        ));

        Ok(())
    }

    /// Enqueue `(ctx, data)` for batched delivery to `peer`.
    pub fn enqueue(
        &self,
        peer: SocketAddr,
        ctx: LaplaceContext,
        data: Vec<u8>,
    ) -> Result<(), MeshAgentError> {
        let tx = self
            .senders
            .get(&peer)
            .ok_or_else(|| MeshAgentError::Quic(format!("no connection to {peer}")))?;
        tx.send((ctx, data))
            .map_err(|_| MeshAgentError::ChannelClosed)
    }

    /// Open a fresh QUIC connection to `addr`.
    async fn open_connection(&self, addr: SocketAddr) -> Result<quinn::Connection, MeshAgentError> {
        let mut ep_guard = self.client_endpoint.lock().await;

        if ep_guard.is_none() {
            let ep = make_client_endpoint(&self.socket_provider)?;
            *ep_guard = Some(ep);
        }

        let ep = ep_guard
            .as_ref()
            .ok_or_else(|| MeshAgentError::Config("endpoint not initialized".into()))?;
        let conn = ep
            .connect(addr, "localhost")
            .map_err(|e| MeshAgentError::Quic(format!("connect config {addr}: {e}")))?
            .await
            .map_err(|e| MeshAgentError::Quic(format!("connect {addr}: {e}")))?;

        tracing::info!(peer = %addr, "MeshAgent: outbound connection established");
        Ok(conn)
    }
}

impl Default for OutboundPool {
    fn default() -> Self {
        Self::new()
    }
}

// ── Batch flusher task ────────────────────────────────────────────────────────

/// Per-peer background task: accumulates messages and flushes to a persistent
/// unidirectional QUIC stream using the Phase 3 wire format.
async fn run_batch_flusher(
    peer: SocketAddr,
    conn: Arc<quinn::Connection>,
    pool: Arc<DashMap<SocketAddr, Arc<quinn::Connection>>>,
    mut rx: mpsc::UnboundedReceiver<OutboundMsg>,
    shared_flags: Arc<AtomicU8>,
    batch_size: usize,
    flush_interval: Duration,
) {
    let mut send_stream: Option<quinn::SendStream> = None;

    loop {
        // ── Accumulate batch ─────────────────────────────────────────────────
        let mut batch: Vec<OutboundMsg> = Vec::with_capacity(batch_size);
        let deadline = tokio::time::Instant::now() + flush_interval;

        loop {
            if batch.len() >= batch_size {
                break;
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            tokio::select! {
                biased;
                msg = rx.recv() => {
                    match msg {
                        Some(item) => batch.push(item),
                        None => {
                            tracing::debug!(peer = %peer, "MeshAgent: batch flusher channel closed");
                            return;
                        }
                    }
                }
                _ = tokio::time::sleep(remaining) => break,
            }
        }

        if batch.is_empty() {
            continue;
        }

        // ── Ensure open stream ────────────────────────────────────────────────
        if send_stream.is_none() {
            let current_conn = pool
                .get(&peer)
                .map(|r| r.clone())
                .unwrap_or_else(|| conn.clone());
            match current_conn.open_uni().await {
                Ok(stream) => {
                    send_stream = Some(stream);
                    tracing::debug!(peer = %peer, "MeshAgent: opened persistent send stream");
                }
                Err(e) => {
                    tracing::warn!(peer = %peer, error = ?e, "MeshAgent: open_uni failed — batch dropped");
                    continue;
                }
            }
        }

        // ── Flush batch with Phase 3 wire format ─────────────────────────────
        let compression_flags = shared_flags.load(Ordering::Relaxed);
        let Some(stream) = send_stream.as_mut() else {
            continue;
        };
        let mut flush_ok = true;

        for (ctx, payload) in &batch {
            // Determine whether to apply LZ4 (Layer 3)
            let use_lz4 =
                (compression_flags & FLAG_LAYER3) != 0 && payload.len() > LZ4_COMPRESSION_THRESHOLD;

            let (final_payload, layer3_active) = if use_lz4 {
                match lz4_compress(payload) {
                    Ok(compressed) => (compressed, true),
                    Err(e) => {
                        tracing::warn!(peer = %peer, error = ?e, "LZ4 compression failed — sending uncompressed");
                        (payload.clone(), false)
                    }
                }
            } else {
                (payload.clone(), false)
            };

            // Build flags byte: always set CTX_FLAG (bit 4) + user compression flags
            // For Layer 3, only set the bit if we actually compressed
            let mut frame_flags: u8 = CTX_FLAG | (compression_flags & (FLAG_LAYER1 | FLAG_LAYER2));
            if layer3_active {
                frame_flags |= FLAG_LAYER3;
            }

            let ctx_bytes = ctx.to_bytes();
            // total_frame_len = 1 (flags) + CONTEXT_BYTES + payload.len()
            let total_frame_len = (1 + CONTEXT_BYTES + final_payload.len()) as u32;

            let ok = stream
                .write_all(&total_frame_len.to_be_bytes())
                .await
                .is_ok()
                && stream.write_all(&[frame_flags]).await.is_ok()
                && stream.write_all(&ctx_bytes).await.is_ok()
                && stream.write_all(&final_payload).await.is_ok();

            if !ok {
                tracing::warn!(
                    peer = %peer,
                    "MeshAgent: stream write failed — will reopen on next flush"
                );
                flush_ok = false;
                break;
            }
        }

        if !flush_ok {
            send_stream = None;
        }

        tracing::debug!(peer = %peer, count = batch.len(), "MeshAgent: batch flushed");
    }
}

// ── TLS helpers ───────────────────────────────────────────────────────────────

pub fn make_client_endpoint(
    socket_provider: &Arc<dyn SocketProvider>,
) -> Result<quinn::Endpoint, MeshAgentError> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{DigitallySignedStruct, SignatureScheme};

    #[derive(Debug)]
    struct SkipVerify;

    impl rustls::client::danger::ServerCertVerifier for SkipVerify {
        fn verify_server_cert(
            &self,
            _: &CertificateDer<'_>,
            _: &[CertificateDer<'_>],
            _: &ServerName<'_>,
            _: &[u8],
            _: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn verify_tls13_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![
                SignatureScheme::RSA_PKCS1_SHA256,
                SignatureScheme::RSA_PKCS1_SHA384,
                SignatureScheme::RSA_PKCS1_SHA512,
                SignatureScheme::ECDSA_NISTP256_SHA256,
                SignatureScheme::ECDSA_NISTP384_SHA384,
                SignatureScheme::ED25519,
            ]
        }
    }

    let client_config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipVerify))
        .with_no_client_auth();

    let quinn_client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(client_config)
            .map_err(|e| MeshAgentError::Config(format!("client TLS config: {e}")))?,
    ));

    let bind_addr: std::net::SocketAddr = "0.0.0.0:0"
        .parse()
        .map_err(|e| MeshAgentError::Config(format!("bind addr parse: {e}")))?;
    let bound = socket_provider
        .bind(bind_addr)
        .map_err(|e| MeshAgentError::Io(format!("client bind: {e}")))?;
    let udp_socket = match bound {
        BoundSocket::Os(s) => {
            s.set_nonblocking(true)
                .map_err(|e| MeshAgentError::Io(format!("set_nonblocking: {e}")))?;
            s
        }
        BoundSocket::Virtual { .. } => {
            return Err(MeshAgentError::Config(
                "virtual socket not yet supported for outbound".into(),
            ));
        }
    };
    let mut endpoint = quinn::Endpoint::new(
        Default::default(),
        None,
        udp_socket,
        Arc::new(quinn::TokioRuntime),
    )
    .map_err(|e| MeshAgentError::Quic(format!("client endpoint: {e}")))?;
    endpoint.set_default_client_config(quinn_client_config);

    Ok(endpoint)
}
