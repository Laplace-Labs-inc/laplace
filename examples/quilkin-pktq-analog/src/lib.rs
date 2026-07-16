// SPDX-License-Identifier: Apache-2.0
//! Customer-owned analog of quilkin's packet-queue push/notify/drain protocol.
//!
//! This is a contract re-expression, not copied quilkin code. The public
//! Route A surface uses only `laplace_sdk::rt`; Route B exercises the same
//! methods through the private async engine.

use std::sync::Arc;

use laplace_sdk::rt::{watch, ModelAsyncMutex};

/// Policy used to select the normal protocol or the explicit fault fixture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WakePolicy {
    /// Every enqueue publishes the watch notification.
    Normal,
    /// Test-only inverse of the push-to-notify discipline.
    #[cfg(feature = "fault-fixture")]
    SuppressOnePushWake,
}

struct QueueState {
    contents: ModelAsyncMutex<QueueContents>,
    notify: watch::Sender<bool>,
    policy: WakePolicy,
}

struct QueueContents {
    packets: Vec<u64>,
    notifications_sent: usize,
}

/// A single-consumer packet queue with a watch notification plane.
pub struct PacketQueue {
    state: Arc<QueueState>,
}

impl PacketQueue {
    /// Creates an empty queue and its sole watch receiver.
    #[must_use]
    pub fn new(policy: WakePolicy) -> (Arc<Self>, watch::Receiver<bool>) {
        let (notify, receiver) = watch::channel(true);
        let state = Arc::new(QueueState {
            contents: ModelAsyncMutex::new(QueueContents {
                packets: Vec::new(),
                notifications_sent: 0,
            }),
            notify,
            policy,
        });
        (Arc::new(Self { state }), receiver)
    }

    /// Pushes one packet, then publishes the availability notification.
    ///
    /// The guard is dropped before `send`, matching the pinned source's short
    /// synchronous critical section even though this analog uses the model
    /// async mutex seam.
    pub async fn enqueue(&self, packet: u64) {
        {
            let mut contents = self.state.contents.lock().await;
            contents.packets.push(packet);
        }

        let should_notify = match self.state.policy {
            WakePolicy::Normal => true,
            #[cfg(feature = "fault-fixture")]
            WakePolicy::SuppressOnePushWake => false,
        };
        if should_notify {
            {
                let mut contents = self.state.contents.lock().await;
                contents.notifications_sent = contents.notifications_sent.saturating_add(1);
            }
            let _ = self.state.notify.send(true);
        }
    }

    /// Publishes the shutdown value after the producer has finished.
    pub async fn shutdown(&self) {
        let _ = self.state.notify.send(false);
    }

    /// Waits for watch changes and drains the queue under the model lock.
    ///
    /// Shutdown is accepted only after the queue is empty. If a packet is
    /// still present when shutdown is observed, the consumer returns to
    /// `changed()`; this preserves the push-to-notify discipline and exposes a
    /// missing notification as a real terminal wait in Route B.
    pub async fn drain(&self, mut receiver: watch::Receiver<bool>) {
        loop {
            if receiver.changed().await.is_err() {
                return;
            }

            let shutdown = !*receiver.borrow();
            let (queue_empty, notification_sent) = {
                let mut contents = self.state.contents.lock().await;
                if shutdown {
                    if contents.packets.is_empty() {
                        (true, false)
                    } else if contents.notifications_sent > 0 {
                        let _drained = std::mem::take(&mut contents.packets);
                        (false, true)
                    } else {
                        (false, false)
                    }
                } else {
                    let _drained = std::mem::take(&mut contents.packets);
                    (true, false)
                }
            };

            if shutdown && (queue_empty || notification_sent) {
                return;
            }
        }
    }
}

/// Route A public capture: one producer, one consumer, and one shutdown task.
#[allow(dead_code)]
#[laplace_sdk::verify(tasks, name = "quilkin_pktq_push_notify_drain")]
fn quilkin_pktq_push_notify_drain(tasks: &mut laplace_sdk::rt::TaskSet) {
    let (queue, receiver) = PacketQueue::new(WakePolicy::Normal);
    let producer_queue = Arc::clone(&queue);
    let producer = tasks.spawn(async move {
        producer_queue.enqueue(7).await;
    });

    let consumer_queue = Arc::clone(&queue);
    tasks.spawn(async move {
        consumer_queue.drain(receiver).await;
    });

    tasks.spawn(async move {
        producer.await;
        queue.shutdown().await;
    });
}
