//! # Pluggable Network Abstractions
//!
//! Defines the three Dependency Injection (DI) traits that enable Distributed Axiom
//! to replace the real OS network stack with an in-memory virtual backend.
//!
//! ## Design Intent
//!
//! Every concrete implementation (`QuicServer`, `ConnectionHandler`, etc.) in
//! `laplace-knul` is constructed with these traits injected, rather than calling
//! `std::net::UdpSocket::bind()` or `SystemTime::now()` directly.  In production
//! the default implementations (`OsSocketProvider`, `WallClockProvider`,
//! `NullInterceptor`) are used and behaviour is identical to today.  In
//! Distributed Axiom mode the Coordinator swaps in virtual implementations
//! without touching any other code.
//!
//! ## Traits
//!
//! | Trait | Default | Axiom override |
//! |-------|---------|--------------|
//! | [`SocketProvider`] | [`OsSocketProvider`] | `VirtualSocketProvider` (Phase 4) |
//! | [`NetworkClockProvider`] | [`WallClockProvider`] | `VirtualClockProvider` (Phase 4) |
//! | [`PacketInterceptor`] | [`NullInterceptor`] | `ChaosInterceptor` (Phase 4) |

use std::net::{SocketAddr, UdpSocket};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

// ============================================================================
// BoundSocket — abstract socket handle
// ============================================================================

/// An abstract socket binding — either an OS UDP socket or an in-memory virtual
/// channel — returned by [`SocketProvider::bind`].
///
/// ## Variants
///
/// * [`BoundSocket::Os`] — real `std::net::UdpSocket`; passed directly to
///   `quinn::Endpoint::new` in production.
/// * [`BoundSocket::Virtual`] — opaque handle for Distributed Axiom simulation.
///   `laplace-knul` downcasts `inner` to
///   `laplace_knul::infrastructure::transport::virtual_socket::VirtualUdpSocket`.
pub enum BoundSocket {
    /// Production: OS-level UDP socket, owned and ready for I/O.
    Os(UdpSocket),

    /// Distributed Axiom: in-memory virtual socket.
    ///
    /// `local_addr` is the address this socket is "bound" to inside the virtual
    /// network.  `inner` is a `Box<VirtualUdpSocket>` (from `laplace-knul`)
    /// type-erased here to avoid a circular dependency with `tokio`.
    Virtual {
        /// The simulated local address of this socket.
        local_addr: SocketAddr,
        /// Opaque [`VirtualUdpSocket`] handle; downcast in `laplace-knul`.
        inner: Box<dyn std::any::Any + Send + Sync>,
    },
}

use crate::domain::transport::types::TransportPacket;
use crate::domain::transport::TransportError;

// ============================================================================
// PacketBuffer — inbound packet alias
// ============================================================================

/// Raw inbound packet buffer flowing from the network layer to the kernel.
///
/// Type alias for [`TransportPacket`] that emphasises the *receive* direction:
/// bytes arrive from a remote peer, get wrapped here, and are handed to
/// the kernel for dispatch.  [`PacketInterceptor::on_receive`] may mutate
/// this buffer (e.g., inject artificial delay metadata) before it enters the
/// processing queue.
pub type PacketBuffer = TransportPacket;

// ============================================================================
// SocketProvider
// ============================================================================

/// Abstracts UDP socket creation so the real OS socket can be replaced by an
/// in-memory channel during Distributed Axiom simulation.
///
/// ## Implementors
///
/// * [`OsSocketProvider`] — production default; calls `std::net::UdpSocket::bind`.
/// * `VirtualSocketProvider` — Phase 4 Distributed Axiom; returns an in-memory
///   pipe that Quinn can drive without touching the OS.
///
/// ## Quinn compatibility
///
/// `quinn::Endpoint::new` accepts a `std::net::UdpSocket` directly, so this
/// trait returns the concrete `UdpSocket` type for now.  A future
/// `VirtualSocketProvider` implementation will emulate a `UdpSocket` via
/// `RawFd`/`socketpair` on Unix or an equivalent shim.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Transport",
        link = "LEP-0013-laplace-interfaces-pluggable_network_chaos"
    )
)]
pub trait SocketProvider: Send + Sync + 'static {
    /// Bind to `addr` and return an abstract [`BoundSocket`].
    ///
    /// * Production callers receive [`BoundSocket::Os`] wrapping a real
    ///   `std::net::UdpSocket` that Quinn consumes directly.
    /// * Distributed Axiom callers receive [`BoundSocket::Virtual`] backed by an
    ///   in-memory `VirtualUdpSocket` that routes packets through
    ///   [`VirtualNetworkRouter`].
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::IoError`] if the underlying bind operation fails
    /// (e.g., address already in use, insufficient privileges).
    fn bind(&self, addr: SocketAddr) -> Result<BoundSocket, TransportError>;
}

/// Production [`SocketProvider`] that delegates to the OS network stack.
///
/// Calls `std::net::UdpSocket::bind(addr)` and maps any I/O error to
/// [`TransportError::IoError`].  This is the default used in all non-simulation
/// contexts.
#[derive(Debug, Clone, Copy, Default)]
pub struct OsSocketProvider;

impl SocketProvider for OsSocketProvider {
    fn bind(&self, addr: SocketAddr) -> Result<BoundSocket, TransportError> {
        let socket = UdpSocket::bind(addr).map_err(|_| TransportError::IoError)?;
        Ok(BoundSocket::Os(socket))
    }
}

// ============================================================================
// NetworkClockProvider
// ============================================================================

/// Abstracts time-of-day so the real wall clock can be replaced by the
/// deterministic [`laplace_core::VirtualClock`] during simulation.
///
/// ## Implementors
///
/// * [`WallClockProvider`] — production default; reads `SystemTime::now()`.
/// * `VirtualClockProvider` — Phase 4 Distributed Axiom; wraps `VirtualClock`
///   so every node in the simulation shares the same deterministic tick.
///
/// ## Why this matters for Distributed Axiom
///
/// Packet timestamps produced by `ConnectionHandler` are the foundation of
/// the global causal order that DPOR reconstructs during replay.  If even one
/// node uses the real wall clock, the timestamp sequence becomes
/// non-deterministic and the Heisenbug becomes unreproducible.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Transport",
        link = "LEP-0013-laplace-interfaces-pluggable_network_chaos"
    )
)]
pub trait NetworkClockProvider: Send + Sync + 'static {
    /// Current time in microseconds since the Unix epoch.
    ///
    /// In production this is a monotonic wall-clock read.  In simulation this
    /// is a logical tick value controlled by the Coordinator.
    fn now_us(&self) -> u64;
}

/// Production [`NetworkClockProvider`] that reads the OS wall clock.
///
/// Uses `std::time::SystemTime::now()` converted to microseconds since the
/// Unix epoch.  Returns `0` if the system clock is set before the epoch
/// (should never occur in practice).
#[derive(Debug, Clone, Copy, Default)]
pub struct WallClockProvider;

impl NetworkClockProvider for WallClockProvider {
    fn now_us(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0)
    }
}

// ============================================================================
// InterceptReason
// ============================================================================

/// Reason why a packet was dropped by a [`PacketInterceptor`].
///
/// Returned as the `Err` variant of [`PacketInterceptor::on_receive`] when the
/// interceptor decides the packet must not reach the kernel.  The transport
/// layer discards the packet and records the reason in its statistics counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterceptReason {
    /// Simulated random packet loss (e.g., 5% drop rate in a chaos scenario).
    PacketDrop,

    /// Simulated network partition: all traffic between two node sets is
    /// severed for the duration of the chaos event.
    NetworkPartition,
}

// ============================================================================
// PacketInterceptor
// ============================================================================

/// Intercepts packets on the receive and send paths, allowing the Distributed
/// Axiom Coordinator to inject deterministic network chaos into every node.
///
/// ## Implementors
///
/// * [`NullInterceptor`] — production default; passes every packet through
///   unchanged with zero overhead.
/// * `ChaosInterceptor` — Phase 4 Distributed Axiom; executes the Coordinator's
///   deterministic chaos script (packet delay, drop, partition) on each packet.
///
/// ## Ordering guarantee
///
/// `on_receive` is called *before* a packet is enqueued into the kernel.
/// `on_send` is called *before* a packet is written to the Quinn stream.
/// Both hooks run synchronously in the transport thread.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Transport",
        link = "LEP-0013-laplace-interfaces-pluggable_network_chaos"
    )
)]
pub trait PacketInterceptor: Send + Sync + 'static {
    /// Inspect or mutate an inbound packet before it reaches the kernel.
    ///
    /// # Arguments
    ///
    /// * `packet` — mutable reference to the inbound buffer; the interceptor
    ///   may modify `timestamp_us` or other fields.
    ///
    /// # Returns
    ///
    /// * `Ok(())` — packet is forwarded normally.
    /// * `Err(reason)` — packet is silently dropped; `reason` is recorded.
    fn on_receive(&self, packet: &mut PacketBuffer) -> Result<(), InterceptReason>;

    /// Inspect an outbound packet and return an artificial send delay.
    ///
    /// # Arguments
    ///
    /// * `packet` — immutable reference to the outbound packet; the interceptor
    ///   must not modify it (only the receive path has mutable access).
    ///
    /// # Returns
    ///
    /// Microseconds to delay before the packet is handed to Quinn.
    /// `0` means send immediately.
    fn on_send(&self, packet: &TransportPacket) -> u64;
}

/// Production [`PacketInterceptor`] that passes every packet through unchanged.
///
/// Both hooks are unconditional no-ops.  The compiler will inline and eliminate
/// them entirely in release builds, ensuring **zero overhead** on the production
/// hot path.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullInterceptor;

impl PacketInterceptor for NullInterceptor {
    #[inline(always)]
    fn on_receive(&self, _packet: &mut PacketBuffer) -> Result<(), InterceptReason> {
        Ok(())
    }

    #[inline(always)]
    fn on_send(&self, _packet: &TransportPacket) -> u64 {
        0
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wall_clock_provider_returns_nonzero() {
        let clock = WallClockProvider;
        assert!(clock.now_us() > 0, "wall clock should be past Unix epoch");
    }

    #[test]
    fn null_interceptor_passes_all_packets() {
        let interceptor = NullInterceptor;
        let mut packet = PacketBuffer::new(vec![1, 2, 3], 1);
        assert!(interceptor.on_receive(&mut packet).is_ok());
        assert_eq!(interceptor.on_send(&packet), 0);
    }

    #[test]
    fn os_socket_provider_binds_loopback() {
        let provider = OsSocketProvider;
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let bound = provider.bind(addr);
        assert!(bound.is_ok(), "loopback bind should succeed");
        assert!(matches!(bound.unwrap(), BoundSocket::Os(_)));
    }

    #[test]
    fn intercept_reason_variants_are_distinct() {
        assert_ne!(
            InterceptReason::PacketDrop,
            InterceptReason::NetworkPartition
        );
    }
}
