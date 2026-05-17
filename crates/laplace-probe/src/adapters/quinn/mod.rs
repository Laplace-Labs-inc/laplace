// SPDX-License-Identifier: Apache-2.0
//! Quinn Transport Adapter
//!
//! Complete Quinn-based QUIC transport implementation of the SovereignTransport
//! trait. Wraps quinn-specific logic and provides clean abstraction boundary.

pub mod handler;
pub mod server;

use async_trait::async_trait;
use dashmap::DashMap;
use laplace_interfaces::domain::transport::pluggable::{
    NetworkClockProvider, NullInterceptor, OsSocketProvider, PacketInterceptor, SocketProvider,
    WallClockProvider,
};
use laplace_interfaces::{
    FfiQuicConfig, LaplaceError, SovereignTransport, TransportHandle, TransportPacket,
    TransportStats,
};
pub use server::QuicServer;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Quinn-based transport implementation of SovereignTransport
///
/// This structure wraps the quinn-specific implementation details and provides
/// a clean abstraction boundary that allows the kernel to interact with transport
/// without depending on quinn, rustls, or other version-specific dependencies.
pub struct QuinnTransport {
    /// Internal server registry for managing multiple instances
    servers: Arc<DashMap<TransportHandle, Arc<QuicServer>>>,
    /// Handle allocator for generating unique server identifiers
    next_handle: Arc<AtomicU64>,
    // ── Pluggable DI fields (Phase 2.2) ──────────────────────────────────────
    /// Socket provider: OS bind in production, in-memory channel in Distributed Axiom
    socket_provider: Arc<dyn SocketProvider>,
    /// Clock provider: wall clock in production, VirtualClock in Distributed Axiom
    clock: Arc<dyn NetworkClockProvider>,
    /// Packet interceptor: no-op in production, ChaosInterceptor in Distributed Axiom
    interceptor: Arc<dyn PacketInterceptor>,
}

impl fmt::Debug for QuinnTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QuinnTransport")
            .field("servers_count", &self.servers.len())
            .field("next_handle", &self.next_handle.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl QuinnTransport {
    /// Create a new Quinn transport factory with all-default production backends.
    pub fn new_production() -> Self {
        Self::new_with_di(
            Arc::new(OsSocketProvider),
            Arc::new(WallClockProvider),
            Arc::new(NullInterceptor),
        )
    }

    /// Create a new Quinn transport factory with injected pluggable backends.
    pub fn new_with_di(
        socket_provider: Arc<dyn SocketProvider>,
        clock: Arc<dyn NetworkClockProvider>,
        interceptor: Arc<dyn PacketInterceptor>,
    ) -> Self {
        Self {
            servers: Arc::new(DashMap::new()),
            next_handle: Arc::new(AtomicU64::new(1)),
            socket_provider,
            clock,
            interceptor,
        }
    }

    /// Allocate the next unique handle
    fn allocate_handle(&self) -> TransportHandle {
        self.next_handle.fetch_add(1, Ordering::SeqCst)
    }

    /// Get reference to a registered server
    fn get_server(&self, handle: TransportHandle) -> Option<Arc<QuicServer>> {
        self.servers.get(&handle).map(|entry| Arc::clone(&entry))
    }
}

impl Default for QuinnTransport {
    fn default() -> Self {
        Self::new_production()
    }
}

#[async_trait]
impl SovereignTransport for QuinnTransport {
    async fn start(&self, config: FfiQuicConfig) -> Result<TransportHandle, LaplaceError> {
        let handle = self.allocate_handle();
        let server = QuicServer::new(
            handle,
            config,
            self.socket_provider.clone(),
            self.clock.clone(),
            self.interceptor.clone(),
        );

        server.start().await?;

        self.servers.insert(handle, Arc::new(server));

        Ok(handle)
    }

    async fn stop(&self, handle: TransportHandle) -> Result<(), LaplaceError> {
        let server = self
            .servers
            .remove(&handle)
            .map(|(_, s)| s)
            .ok_or(LaplaceError::InvalidPointer)?;

        server.stop().await?;

        Ok(())
    }

    async fn dequeue_packet(
        &self,
        handle: TransportHandle,
    ) -> Result<Option<TransportPacket>, LaplaceError> {
        let server = self
            .get_server(handle)
            .ok_or(LaplaceError::InvalidPointer)?;

        let packet = server
            .packet_queue()
            .try_dequeue()
            .await
            .map(|pb| pb.into_transport_packet());

        Ok(packet)
    }

    async fn get_stats(&self, handle: TransportHandle) -> Result<TransportStats, LaplaceError> {
        let server = self
            .get_server(handle)
            .ok_or(LaplaceError::InvalidPointer)?;

        let stats = server.get_stats().into_transport_stats();

        Ok(stats)
    }

    async fn enqueue_send_packet(
        &self,
        handle: TransportHandle,
        packet: TransportPacket,
    ) -> Result<(), LaplaceError> {
        let server = self
            .get_server(handle)
            .ok_or(LaplaceError::InvalidPointer)?;

        server.send_packet(packet).await
    }

    async fn is_running(&self, handle: TransportHandle) -> bool {
        self.get_server(handle)
            .map(|server| server.is_running())
            .unwrap_or(false)
    }
}
