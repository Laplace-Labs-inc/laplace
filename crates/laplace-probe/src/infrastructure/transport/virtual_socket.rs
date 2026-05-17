// SPDX-License-Identifier: Apache-2.0
//! Virtual Network Socket — Distributed Axiom in-memory routing layer.
//!
//! Provides a fully in-memory UDP-like network stack that replaces the real OS
//! socket when running a Distributed Axiom simulation.  No OS port is ever
//! opened; packets travel through `tokio::sync::mpsc` channels routed by the
//! [`VirtualNetworkRouter`] central exchange.
//!
//! ## Architecture
//!
//! ```text
//!  Node A                      VirtualNetworkRouter
//!  ┌──────────────────┐        ┌───────────────────────────────┐
//!  │ VirtualUdpSocket │──send─▶│ DashMap<SocketAddr, Sender<>> │──▶ Node B
//!  │  (Receiver)      │◀─recv──│                               │◀── Node C
//!  └──────────────────┘        └───────────────────────────────┘
//! ```
//!
//! ## Usage
//!
//! 1. Create a shared [`VirtualNetworkRouter`] (one per simulation).
//! 2. Wrap it in a [`VirtualSocketProvider`].
//! 3. Inject the provider as the [`SocketProvider`] for each node's `QuicServer`.
//! 4. When `QuicServer::start()` calls `socket_provider.bind(addr)`, the provider
//!    registers the node in the router and returns a `BoundSocket::Virtual`.

use std::net::SocketAddr;
use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::mpsc;

use laplace_interfaces::domain::transport::pluggable::{BoundSocket, SocketProvider};
use laplace_interfaces::domain::transport::TransportError;

// ============================================================================
// VirtualPacket
// ============================================================================

/// A datagram travelling through the virtual network.
///
/// Carries the source address alongside the raw payload so that the receiver
/// can implement address-based filtering or routing.
#[derive(Debug, Clone)]
pub struct VirtualPacket {
    /// Source address of the sender node.
    pub src: SocketAddr,
    /// Raw UDP payload bytes.
    pub data: Vec<u8>,
}

// ============================================================================
// VirtualNetworkRouter
// ============================================================================

/// Central in-memory packet switch for a Distributed Axiom simulation.
///
/// Maintains a routing table of `SocketAddr → Sender<VirtualPacket>`.  Any
/// [`VirtualUdpSocket`] that wants to send a datagram looks up the destination
/// address here and pushes the packet into the corresponding channel.
///
/// The router is typically shared across all nodes in a simulation via
/// `Arc<VirtualNetworkRouter>`.
#[derive(Debug, Default)]
pub struct VirtualNetworkRouter {
    routes: DashMap<SocketAddr, mpsc::UnboundedSender<VirtualPacket>>,
}

impl VirtualNetworkRouter {
    /// Create a new empty router.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            routes: DashMap::new(),
        })
    }

    /// Register a virtual socket's sender under `addr`.
    ///
    /// Called by [`VirtualSocketProvider::bind`] when a node binds to a
    /// simulated address.  Replaces any existing registration for the same
    /// address (idempotent re-registration).
    pub fn register(&self, addr: SocketAddr, tx: mpsc::UnboundedSender<VirtualPacket>) {
        self.routes.insert(addr, tx);
    }

    /// Remove the registration for `addr`.
    ///
    /// Called when a virtual socket is dropped or a node shuts down.
    pub fn unregister(&self, addr: &SocketAddr) {
        self.routes.remove(addr);
    }

    /// Route a packet to the node bound at `dest`.
    ///
    /// # Errors
    ///
    /// Returns `Err(String)` if no node is registered at `dest`, or if the
    /// receiver end has been dropped (the destination node has shut down).
    pub fn send_to(&self, dest: &SocketAddr, packet: VirtualPacket) -> Result<(), String> {
        match self.routes.get(dest) {
            Some(tx) => tx.send(packet).map_err(|e| e.to_string()),
            None => Err(format!("VirtualNetworkRouter: no route to {}", dest)),
        }
    }

    /// Number of currently registered nodes.
    pub fn node_count(&self) -> usize {
        self.routes.len()
    }
}

// ============================================================================
// VirtualUdpSocket
// ============================================================================

/// An in-memory UDP-like socket for use inside a Distributed Axiom simulation.
///
/// Holds the receive end of an `mpsc` channel and a reference to the shared
/// [`VirtualNetworkRouter`] so it can route outgoing packets to other nodes.
///
/// Produced exclusively by [`VirtualSocketProvider::bind`].
pub struct VirtualUdpSocket {
    /// The simulated local address this socket is "bound" to.
    local_addr: SocketAddr,

    /// Receive channel — packets sent to `local_addr` arrive here.
    rx: mpsc::UnboundedReceiver<VirtualPacket>,

    /// Shared router — used to deliver outgoing packets.
    router: Arc<VirtualNetworkRouter>,
}

impl VirtualUdpSocket {
    /// The simulated local address of this socket.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Receive the next incoming packet, waiting until one is available.
    ///
    /// Returns `None` if all senders for this socket have been dropped
    /// (i.e., the simulation is shutting down).
    pub async fn recv_from(&mut self) -> Option<VirtualPacket> {
        self.rx.recv().await
    }

    /// Send a packet to `dest` through the router.
    ///
    /// # Errors
    ///
    /// Returns `Err(String)` if `dest` has no registered node or its receiver
    /// has been dropped.
    pub fn send_to(&self, dest: SocketAddr, data: Vec<u8>) -> Result<(), String> {
        let packet = VirtualPacket {
            src: self.local_addr,
            data,
        };
        self.router.send_to(&dest, packet)
    }
}

impl Drop for VirtualUdpSocket {
    /// Unregister from the router when the socket is dropped so stale entries
    /// do not accumulate in the routing table.
    fn drop(&mut self) {
        self.router.unregister(&self.local_addr);
    }
}

// ============================================================================
// VirtualSocketProvider
// ============================================================================

/// [`SocketProvider`] that allocates in-memory sockets backed by a shared
/// [`VirtualNetworkRouter`] instead of opening real OS ports.
///
/// Inject this as the `socket_provider` argument of `QuicServer::new()` to run
/// a node inside a Distributed Axiom simulation.
///
/// ## Example
///
/// ```ignore
/// let router = VirtualNetworkRouter::new();
/// let provider = VirtualSocketProvider::new(router.clone());
/// let server = QuicServer::new(
///     handle,
///     config,
///     Arc::new(provider),
///     clock,
///     interceptor,
/// );
/// ```
#[derive(Clone, Debug)]
pub struct VirtualSocketProvider {
    router: Arc<VirtualNetworkRouter>,
}

impl VirtualSocketProvider {
    /// Create a new provider backed by `router`.
    ///
    /// Multiple providers sharing the same router form a single virtual LAN —
    /// all nodes can reach each other by their simulated `SocketAddr`.
    pub fn new(router: Arc<VirtualNetworkRouter>) -> Self {
        Self { router }
    }
}

impl SocketProvider for VirtualSocketProvider {
    /// "Bind" to `addr` by registering a channel in the router.
    ///
    /// Returns a [`BoundSocket::Virtual`] whose `inner` field holds a
    /// [`VirtualUdpSocket`] downcasted in `laplace-knul` when Quinn integration
    /// (Phase 4) is complete.
    fn bind(&self, addr: SocketAddr) -> Result<BoundSocket, TransportError> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.router.register(addr, tx);

        let socket = VirtualUdpSocket {
            local_addr: addr,
            rx,
            router: self.router.clone(),
        };

        Ok(BoundSocket::Virtual {
            local_addr: addr,
            inner: Box::new(socket),
        })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_register_and_node_count() {
        let router = VirtualNetworkRouter::new();
        assert_eq!(router.node_count(), 0);

        let (tx, _rx) = mpsc::unbounded_channel();
        let addr: SocketAddr = "127.0.0.1:9001".parse().unwrap();
        router.register(addr, tx);
        assert_eq!(router.node_count(), 1);

        router.unregister(&addr);
        assert_eq!(router.node_count(), 0);
    }

    #[tokio::test]
    async fn router_routes_packet_between_nodes() {
        let router = VirtualNetworkRouter::new();

        let addr_a: SocketAddr = "127.0.0.1:9001".parse().unwrap();
        let addr_b: SocketAddr = "127.0.0.1:9002".parse().unwrap();

        let (tx_a, _rx_a) = mpsc::unbounded_channel();
        let (tx_b, mut rx_b) = mpsc::unbounded_channel();
        router.register(addr_a, tx_a);
        router.register(addr_b, tx_b);

        let packet = VirtualPacket {
            src: addr_a,
            data: vec![1, 2, 3],
        };
        router.send_to(&addr_b, packet).expect("route must exist");

        let received = rx_b.recv().await.expect("must receive packet");
        assert_eq!(received.src, addr_a);
        assert_eq!(received.data, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn virtual_socket_provider_bind_and_send() {
        let router = VirtualNetworkRouter::new();
        let provider = VirtualSocketProvider::new(router.clone());

        let addr_a: SocketAddr = "127.0.0.1:9001".parse().unwrap();
        let addr_b: SocketAddr = "127.0.0.1:9002".parse().unwrap();

        // Bind both sockets
        let bound_a = provider.bind(addr_a).expect("bind A must succeed");
        let bound_b = provider.bind(addr_b).expect("bind B must succeed");

        // Both must be Virtual variant
        assert!(matches!(bound_a, BoundSocket::Virtual { .. }));
        assert!(matches!(bound_b, BoundSocket::Virtual { .. }));

        assert_eq!(router.node_count(), 2);
    }

    #[tokio::test]
    async fn virtual_socket_send_recv() {
        let router = VirtualNetworkRouter::new();

        let addr_a: SocketAddr = "127.0.0.1:9001".parse().unwrap();
        let addr_b: SocketAddr = "127.0.0.1:9002".parse().unwrap();

        let (tx_a, _rx_a) = mpsc::unbounded_channel::<VirtualPacket>();
        let (tx_b, mut rx_b) = mpsc::unbounded_channel::<VirtualPacket>();
        router.register(addr_a, tx_a);
        router.register(addr_b, tx_b);

        // Manually construct a VirtualUdpSocket for node A (sender side only)
        let (tx_dummy, rx_a_sock) = mpsc::unbounded_channel::<VirtualPacket>();
        router.register(addr_a, tx_dummy); // overwrite with dummy

        let socket_a = VirtualUdpSocket {
            local_addr: addr_a,
            rx: rx_a_sock,
            router: router.clone(),
        };

        socket_a
            .send_to(addr_b, vec![42, 43])
            .expect("send must succeed");

        let pkt = rx_b.recv().await.expect("must receive");
        assert_eq!(pkt.src, addr_a);
        assert_eq!(pkt.data, vec![42, 43]);
    }
}
