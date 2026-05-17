//! MeshAgent — Bidirectional QUIC Mesh Node
//!
//! Combines inbound QUIC server with outbound connection pool.
//!
//! ## Phase 3 additions
//!
//! - `openapi_url` on the builder — if provided, fetches the OpenAPI spec on
//!   startup and builds the static [`StaticDictionary`] for Layer 1 encoding.
//! - `set_compression_flags` — enables Layer 1/2/3 compression on outbound frames.
//! - The `recv` method now returns `(Option<LaplaceContext>, Vec<u8>, flags: u8)`.

use dashmap::DashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

use laplace_interfaces::domain::transport::pluggable::{
    NetworkClockProvider, NullInterceptor, OsSocketProvider, WallClockProvider,
};
use laplace_interfaces::ProbeConfig;

use crate::domain::context::LaplaceContext;
use crate::domain::wire::StaticDictionary;

pub mod inbound;
pub mod outbound;

use self::inbound::{InboundLoop, InboundMsg};
use self::outbound::OutboundPool;

// ── Handle-based registry (no OnceLock) ──────────────────────────────────────

/// Thread-safe registry for `MeshAgent` instances using u64 handles.
pub struct MeshAgentRegistry {
    agents: DashMap<u64, Arc<MeshAgent>>,
    next_handle: AtomicU64,
}

impl MeshAgentRegistry {
    pub fn new() -> Self {
        Self {
            agents: DashMap::new(),
            next_handle: AtomicU64::new(1),
        }
    }

    pub fn register(&self, agent: MeshAgent) -> u64 {
        let handle = self.next_handle.fetch_add(1, Ordering::SeqCst);
        self.agents.insert(handle, Arc::new(agent));
        handle
    }

    pub fn get(&self, handle: u64) -> Option<Arc<MeshAgent>> {
        self.agents.get(&handle).map(|r| r.clone())
    }

    pub fn remove(&self, handle: u64) -> Option<Arc<MeshAgent>> {
        self.agents.remove(&handle).map(|(_, v)| v)
    }
}

impl Default for MeshAgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── MeshAgent ─────────────────────────────────────────────────────────────────

/// Bidirectional QUIC Mesh Node.
///
/// Accepts inbound connections (QUIC server) and maintains a pool of persistent
/// outbound connections with batched, optionally compressed frame delivery.
pub struct MeshAgent {
    _inbound: InboundLoop,
    outbound: Arc<OutboundPool>,
    inbound_rx: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<InboundMsg>>>,
    context: Arc<RwLock<LaplaceContext>>,
    clock: Arc<dyn NetworkClockProvider>,
    /// Static dictionary built from OpenAPI auto-discovery (Phase 3 Layer 1).
    static_dict: Arc<RwLock<StaticDictionary>>,
}

impl MeshAgent {
    pub fn builder() -> MeshAgentBuilder {
        MeshAgentBuilder::default()
    }

    /// Connect to a peer and open a persistent outbound connection.
    pub async fn connect_peer(&self, addr: SocketAddr) -> Result<(), MeshAgentError> {
        self.outbound.connect(addr).await
    }

    /// Enqueue `data` for batched delivery to `peer`.
    ///
    /// Auto-stamps `virtual_clock_ns` from the injected clock and increments
    /// the Lamport tick.
    pub async fn send_to_peer(
        &self,
        peer: SocketAddr,
        data: Vec<u8>,
    ) -> Result<(), MeshAgentError> {
        let ctx = {
            let mut ctx = self.context.write().await;
            ctx.virtual_clock_ns = self.clock.now_us().saturating_mul(1_000);
            ctx.lamport_tick = ctx.lamport_tick.wrapping_add(1);
            *ctx
        };
        self.outbound.enqueue(peer, ctx, data)
    }

    /// Update the active [`LaplaceContext`] for subsequent outbound frames.
    pub async fn inject_context(&self, ctx: LaplaceContext) {
        *self.context.write().await = ctx;
    }

    /// Set the active compression layer flags for outbound frames.
    ///
    /// `flags` is a bitmask of `FLAG_LAYER1 | FLAG_LAYER2 | FLAG_LAYER3`.
    pub fn set_compression_flags(&self, flags: u8) {
        self.outbound.set_compression_flags(flags);
    }

    /// Read the current active compression flags.
    pub fn compression_flags(&self) -> u8 {
        self.outbound.compression_flags()
    }

    /// Receive the next inbound frame.
    ///
    /// Returns `(Option<LaplaceContext>, payload: Vec<u8>, flags: u8)`.
    /// The `flags` byte tells the caller which layers were active:
    /// - `FLAG_LAYER1` (0x01): payload starts with a VarInt route_id
    /// - `FLAG_LAYER2` (0x02): payload tokens use dynamic dict IDs
    /// - `FLAG_LAYER3` (0x04): payload was LZ4 compressed (already decompressed)
    pub async fn recv(&self) -> Option<InboundMsg> {
        let mut rx = self.inbound_rx.lock().await;
        rx.recv().await
    }

    /// Read-only access to the static dictionary (Layer 1).
    pub async fn static_dict(&self) -> tokio::sync::RwLockReadGuard<'_, StaticDictionary> {
        self.static_dict.read().await
    }

    /// Read a snapshot of the active context.
    pub async fn current_context(&self) -> LaplaceContext {
        *self.context.read().await
    }
}

// ── Builder ───────────────────────────────────────────────────────────────────

/// Builder for [`MeshAgent`].
pub struct MeshAgentBuilder {
    bind_addr: Option<SocketAddr>,
    initial_context: LaplaceContext,
    clock: Option<Arc<dyn NetworkClockProvider>>,
    /// Phase 3: OpenAPI URL for automatic Layer 1 static dictionary construction.
    openapi_url: Option<String>,
    /// Phase 3: initial compression layer flags.
    compression_flags: u8,
    /// Global probe config — drives batch_size, flush_interval, max_frame_len.
    probe_config: ProbeConfig,
}

#[allow(clippy::derivable_impls)]
impl Default for MeshAgentBuilder {
    fn default() -> Self {
        Self {
            bind_addr: None,
            initial_context: LaplaceContext::default(),
            clock: None,
            openapi_url: None,
            compression_flags: 0,
            probe_config: ProbeConfig::default(),
        }
    }
}

impl MeshAgentBuilder {
    /// Set the local address to listen on for inbound connections.
    pub fn bind_addr(mut self, addr: SocketAddr) -> Self {
        self.bind_addr = Some(addr);
        self
    }

    /// Set an initial [`LaplaceContext`].
    pub fn initial_context(mut self, ctx: LaplaceContext) -> Self {
        self.initial_context = ctx;
        self
    }

    /// Override the clock provider (default: `WallClockProvider`).
    pub fn clock(mut self, clock: Arc<dyn NetworkClockProvider>) -> Self {
        self.clock = Some(clock);
        self
    }

    /// Phase 3 — Layer 1 OpenAPI auto-discovery.
    ///
    /// If `url` is provided, `build()` will fetch the OpenAPI spec from that
    /// URL and populate the [`StaticDictionary`] with all `(METHOD, path)` pairs
    /// before the agent starts accepting connections.
    ///
    /// IDs `0x0001–0x3FFF` are reserved for this static layer.
    pub fn openapi_url(mut self, url: impl Into<String>) -> Self {
        self.openapi_url = Some(url.into());
        self
    }

    /// Phase 3 — set initial compression layer flags.
    ///
    /// Use `FLAG_LAYER1 | FLAG_LAYER2 | FLAG_LAYER3` constants from `outbound`.
    pub fn compression_flags(mut self, flags: u8) -> Self {
        self.compression_flags = flags;
        self
    }

    /// Inject a `ProbeConfig` to control batch_size, flush_interval_ms, and max_frame_len.
    pub fn probe_config(mut self, cfg: ProbeConfig) -> Self {
        self.probe_config = cfg;
        self
    }

    /// Build and start the [`MeshAgent`].
    pub async fn build(self) -> Result<MeshAgent, MeshAgentError> {
        let bind_addr = self
            .bind_addr
            .ok_or(MeshAgentError::Config("bind_addr is required".into()))?;

        let clock: Arc<dyn NetworkClockProvider> =
            self.clock.unwrap_or_else(|| Arc::new(WallClockProvider));

        // Phase 3: OpenAPI auto-discovery → static dictionary
        let static_dict = if let Some(ref url) = self.openapi_url {
            tracing::info!(url = %url, "MeshAgent: fetching OpenAPI spec for Layer 1 dictionary");
            match crate::domain::wire::fetch_and_build_dictionary(url).await {
                Ok(dict) => {
                    tracing::info!(
                        entries = dict.len(),
                        "MeshAgent: Layer 1 static dictionary built from OpenAPI"
                    );
                    dict
                }
                Err(e) => {
                    tracing::warn!(error = ?e, "MeshAgent: OpenAPI fetch failed — using empty dictionary");
                    StaticDictionary::new()
                }
            }
        } else {
            StaticDictionary::new()
        };

        let (inbound_tx, inbound_rx) = tokio::sync::mpsc::unbounded_channel::<InboundMsg>();

        let inbound = InboundLoop::start_with_config(
            bind_addr,
            inbound_tx,
            &self.probe_config,
            Arc::new(OsSocketProvider),
            Arc::new(NullInterceptor),
        )
        .await?;
        let outbound = Arc::new(OutboundPool::with_config(&self.probe_config));

        // Apply initial compression flags
        if self.compression_flags != 0 {
            outbound.set_compression_flags(self.compression_flags);
        }

        Ok(MeshAgent {
            _inbound: inbound,
            outbound,
            inbound_rx: Arc::new(Mutex::new(inbound_rx)),
            context: Arc::new(RwLock::new(self.initial_context)),
            clock,
            static_dict: Arc::new(RwLock::new(static_dict)),
        })
    }
}

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum MeshAgentError {
    Config(String),
    Io(String),
    Quic(String),
    ChannelClosed,
}

impl std::fmt::Display for MeshAgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(m) => write!(f, "MeshAgent config error: {m}"),
            Self::Io(m) => write!(f, "MeshAgent I/O error: {m}"),
            Self::Quic(m) => write!(f, "MeshAgent QUIC error: {m}"),
            Self::ChannelClosed => write!(f, "MeshAgent channel closed"),
        }
    }
}

impl std::error::Error for MeshAgentError {}
