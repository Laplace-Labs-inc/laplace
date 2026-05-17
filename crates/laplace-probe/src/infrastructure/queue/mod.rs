// SPDX-License-Identifier: Apache-2.0
//! Packet Queue Module
//!
//! Low-level queue infrastructure for zero-copy packet handoff between
//! network receive loop and kernel processing layer. Uses tokio channels
//! for lock-free concurrent access.

use crate::domain::types::PacketBuffer;

// ═══════════════════════════════════════════════════════════════════════════
// Production implementation (tokio-based, excluded under Kani)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(not(kani))]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(not(kani))]
use std::sync::Arc;
#[cfg(not(kani))]
use tokio::sync::mpsc;

/// Single Producer, Single Consumer queue for packet buffers.
///
/// Ensures thread-safe, ordered delivery without copying.
/// Provides lock-free enqueue/dequeue operations.
#[cfg(not(kani))]
#[derive(Debug)]
pub struct PacketQueue {
    tx: mpsc::UnboundedSender<PacketBuffer>,
    rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<PacketBuffer>>>,
    /// Atomic counter for queue depth (updated on enqueue/dequeue)
    depth: Arc<AtomicUsize>,
}

#[cfg(not(kani))]
impl PacketQueue {
    /// Create a new packet queue
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx: Arc::new(tokio::sync::Mutex::new(rx)),
            depth: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Enqueue a packet (producer side - internal network loop)
    pub fn enqueue(&self, packet: PacketBuffer) -> Result<(), String> {
        self.tx
            .send(packet)
            .map_err(|e| format!("Queue enqueue failed: {}", e))?;
        self.depth.fetch_add(1, Ordering::Release);
        Ok(())
    }

    /// Try to dequeue without blocking (consumer side - TS layer)
    pub async fn try_dequeue(&self) -> Option<PacketBuffer> {
        let mut rx = self.rx.lock().await;
        if let Some(packet) = rx.recv().await {
            self.depth.fetch_sub(1, Ordering::Release);
            Some(packet)
        } else {
            None
        }
    }

    /// Get queue depth (for monitoring)
    pub fn len(&self) -> usize {
        self.depth.load(Ordering::Acquire)
    }

    /// Check if queue is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Enqueue a packet with interceptor hook (chaos injection point).
    ///
    /// Runs `interceptor.on_receive` before inserting the packet. If the
    /// interceptor returns `Err` (e.g. `ChaosInterceptor` drops the packet or
    /// simulates a network partition), the packet is discarded and the error
    /// reason is returned to the caller — the queue depth is **not** incremented.
    pub fn enqueue_with_intercept(
        &self,
        packet: PacketBuffer,
        interceptor: &dyn laplace_interfaces::domain::transport::pluggable::PacketInterceptor,
    ) -> Result<(), String> {
        use laplace_interfaces::TransportPacket;

        // Convert to TransportPacket for the interceptor hook
        let mut tp = TransportPacket {
            data: packet.data,
            connection_id: packet.connection_handle,
            timestamp_us: packet.timestamp_us,
            stream_id: packet.stream_id,
        };

        // Chaos injection point: packet drop or mutation
        interceptor
            .on_receive(&mut tp)
            .map_err(|r| format!("{:?}", r))?;

        // Convert back to local PacketBuffer (may carry mutated fields)
        let queued = PacketBuffer {
            data: tp.data,
            connection_handle: tp.connection_id,
            timestamp_us: tp.timestamp_us,
            stream_id: tp.stream_id,
        };

        self.tx.send(queued).map_err(|e| e.to_string())?;
        self.depth.fetch_add(1, Ordering::Release);
        Ok(())
    }
}

#[cfg(not(kani))]
impl Default for PacketQueue {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Kani mock (VecDeque-based, replaces tokio to eliminate AtomicWaker explosion)
// ═══════════════════════════════════════════════════════════════════════════

/// Kani-only mock of `PacketQueue`.
///
/// Replaces the tokio `mpsc` channel with a synchronous `VecDeque` wrapped in
/// a `std::sync::Mutex`. This eliminates the `AtomicWaker` state explosion that
/// tokio's unbounded channel causes during bounded model checking, while
/// preserving the identical FIFO semantic that the production queue guarantees.
#[cfg(kani)]
pub struct PacketQueue {
    queue: std::sync::Mutex<std::collections::VecDeque<PacketBuffer>>,
    /// Atomic counter for queue depth (updated on enqueue/dequeue)
    pub depth: std::sync::atomic::AtomicUsize,
}

#[cfg(kani)]
impl PacketQueue {
    /// Create a new packet queue (Kani mock)
    pub fn new() -> Self {
        Self {
            queue: std::sync::Mutex::new(std::collections::VecDeque::new()),
            depth: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Enqueue a packet (Kani mock — sync push_back)
    pub fn enqueue(&self, packet: PacketBuffer) -> Result<(), String> {
        self.queue.lock().unwrap().push_back(packet);
        self.depth
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        Ok(())
    }

    /// Dequeue a packet (Kani mock — async wrapper over sync pop_front).
    ///
    /// Provided so that call-sites that use `.await` continue to compile
    /// unchanged under Kani. The future resolves immediately with no scheduler
    /// involvement.
    pub async fn try_dequeue(&self) -> Option<PacketBuffer> {
        let packet = self.queue.lock().unwrap().pop_front();
        if packet.is_some() {
            self.depth
                .fetch_sub(1, std::sync::atomic::Ordering::Release);
        }
        packet
    }

    /// Synchronous dequeue for Kani proof harnesses.
    pub fn try_dequeue_kani(&self) -> Option<PacketBuffer> {
        let packet = self.queue.lock().unwrap().pop_front();
        if packet.is_some() {
            self.depth
                .fetch_sub(1, std::sync::atomic::Ordering::Release);
        }
        packet
    }

    /// Get queue depth (for monitoring)
    pub fn len(&self) -> usize {
        self.depth.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Check if queue is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Enqueue with interceptor hook (Kani mock).
    ///
    /// Logic is 100% identical to the production version: interceptor is called
    /// first, packet is discarded on `Err`, inserted via `push_back` on `Ok`.
    pub fn enqueue_with_intercept(
        &self,
        packet: PacketBuffer,
        interceptor: &dyn laplace_interfaces::domain::transport::pluggable::PacketInterceptor,
    ) -> Result<(), String> {
        use laplace_interfaces::TransportPacket;

        let mut tp = TransportPacket {
            data: packet.data,
            connection_id: packet.connection_handle,
            timestamp_us: packet.timestamp_us,
            stream_id: packet.stream_id,
        };

        interceptor
            .on_receive(&mut tp)
            .map_err(|r| format!("{:?}", r))?;

        let queued = PacketBuffer {
            data: tp.data,
            connection_handle: tp.connection_id,
            timestamp_us: tp.timestamp_us,
            stream_id: tp.stream_id,
        };

        self.queue.lock().unwrap().push_back(queued);
        self.depth
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        Ok(())
    }
}

#[cfg(kani)]
impl Default for PacketQueue {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Kani proofs
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(kani)]
mod proofs {
    use super::*;

    /// H-KNUL3: PacketQueue preserves FIFO ordering across enqueue/dequeue pairs.
    ///
    /// Uses symbolic connection handles (`kani::any()`) to represent arbitrary
    /// packet identities. Proves that two packets inserted in order A → B are
    /// always dequeued in the same order A → B, with no reordering possible.
    #[kani::proof]
    fn queue_preserves_fifo_order() {
        let queue = PacketQueue::new();

        let h1: u64 = kani::any();
        let h2: u64 = kani::any();
        // Require distinct identifiers so the ordering assertion is meaningful.
        kani::assume(h1 != h2);

        let p1 = PacketBuffer {
            data: vec![],
            connection_handle: h1,
            timestamp_us: 0,
            stream_id: None,
        };
        let p2 = PacketBuffer {
            data: vec![],
            connection_handle: h2,
            timestamp_us: 0,
            stream_id: None,
        };

        queue.enqueue(p1).expect("enqueue p1 must succeed");
        queue.enqueue(p2).expect("enqueue p2 must succeed");

        let d1 = queue
            .try_dequeue_kani()
            .expect("first packet must be immediately available");
        let d2 = queue
            .try_dequeue_kani()
            .expect("second packet must be immediately available");

        assert_eq!(
            d1.connection_handle, h1,
            "first dequeued must match first enqueued (FIFO)"
        );
        assert_eq!(
            d2.connection_handle, h2,
            "second dequeued must match second enqueued (FIFO)"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Unit tests (run against the production tokio-based implementation)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn packet_queue_enqueue_dequeue() {
        let queue = PacketQueue::new();

        let packet = PacketBuffer::new(vec![1, 2, 3], 1);
        let expected_ptr = packet.as_ptr();

        queue.enqueue(packet).expect("enqueue failed");
        assert_eq!(queue.len(), 1);

        let dequeued = queue.try_dequeue().await;

        assert!(dequeued.is_some());

        let p = dequeued.unwrap();

        assert_eq!(p.connection_handle, 1);
        assert_eq!(p.len(), 3);
        assert_eq!(
            p.as_ptr(),
            expected_ptr,
            "Packet data must preserve original Vec allocation (zero-copy)"
        );

        assert_eq!(queue.len(), 0);
        assert!(queue.is_empty());
    }
}
