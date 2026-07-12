// SPDX-License-Identifier: Apache-2.0
//! Customer-owned capacity-one lease/idle-pool protocol.
//!
//! This is intentionally a small protocol written for the PoC. It is not a
//! copied or reduced third-party pool implementation. The only Laplace
//! integration is the explicit public `laplace_sdk::rt` seam used below.

use std::sync::Arc;
use std::time::Duration;

use laplace_sdk::rt::{mpsc, time, ModelAsyncMutex, ModelAsyncNotify};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WakePolicy {
    Normal,
    #[cfg(feature = "fault-fixture")]
    SuppressOneReleaseWake,
}

struct PoolState {
    leased: bool,
    generation: u64,
    idle_generation: Option<u64>,
    idle_expired: bool,
}

enum ManagerCommand {
    ArmIdle { generation: u64 },
    Shutdown,
}

struct PoolInner {
    state: ModelAsyncMutex<PoolState>,
    waiter_wake: ModelAsyncNotify,
    waiter_parked: ModelAsyncNotify,
    idle_wake: ModelAsyncNotify,
    idle_watch: ModelAsyncNotify,
    owner_done: Arc<ModelAsyncNotify>,
    command_tx: ModelAsyncMutex<Option<mpsc::Sender<ManagerCommand>>>,
    idle_timeout: Duration,
    wake_policy: WakePolicy,
}

/// A capacity-one lease pool with an explicit idle-timeout manager.
pub struct LeasePool {
    inner: Arc<PoolInner>,
}

/// The initially leased capacity-one slot returned to the owner task.
pub struct Lease {
    inner: Arc<PoolInner>,
    generation: u64,
}

/// Owns the bounded manager command receiver and exits on `Shutdown`.
pub struct LeaseManager {
    inner: Arc<PoolInner>,
    commands: mpsc::Receiver<ManagerCommand>,
}

impl LeasePool {
    /// Creates a capacity-one pool, an initial owner lease, and its manager.
    #[must_use]
    pub fn new(
        idle_timeout: Duration,
        wake_policy: WakePolicy,
    ) -> (Arc<Self>, Lease, LeaseManager) {
        assert!(!idle_timeout.is_zero(), "idle timeout must be non-zero");

        let (command_tx, command_rx) = mpsc::channel(4);
        let inner = Arc::new(PoolInner {
            state: ModelAsyncMutex::new(PoolState {
                leased: true,
                generation: 1,
                idle_generation: None,
                idle_expired: false,
            }),
            waiter_wake: ModelAsyncNotify::new(),
            waiter_parked: ModelAsyncNotify::new(),
            idle_wake: ModelAsyncNotify::new(),
            idle_watch: ModelAsyncNotify::new(),
            owner_done: Arc::new(ModelAsyncNotify::new()),
            command_tx: ModelAsyncMutex::new(Some(command_tx)),
            idle_timeout,
            wake_policy,
        });

        let pool = Arc::new(Self {
            inner: Arc::clone(&inner),
        });
        let lease = Lease {
            inner: Arc::clone(&inner),
            generation: 1,
        };
        let manager = LeaseManager {
            inner,
            commands: command_rx,
        };
        (pool, lease, manager)
    }

    /// Acquires the sole slot, waiting on the protocol's notify plane.
    pub async fn acquire(&self) -> Lease {
        loop {
            let generation = {
                let mut state = self.inner.state.lock().await;
                if state.leased {
                    None
                } else {
                    state.leased = true;
                    state.generation = state.generation.saturating_add(1);
                    Some(state.generation)
                }
            };

            if let Some(generation) = generation {
                self.inner.idle_wake.notify_one();
                return Lease {
                    inner: Arc::clone(&self.inner),
                    generation,
                };
            }

            self.inner.waiter_parked.notify_one();
            self.inner.waiter_wake.notified().await;
        }
    }

    /// Waits until an acquire attempt has actually parked on the waiter.
    pub async fn wait_until_waiter_is_parked(&self) {
        self.inner.waiter_parked.notified().await;
    }

    /// Returns the supervisor's completion signal used by both routes.
    #[must_use]
    pub fn owner_done_signal(&self) -> Arc<ModelAsyncNotify> {
        Arc::clone(&self.inner.owner_done)
    }

    /// Waits for the manager's first idle observation to finish.
    pub async fn watch_idle(self: Arc<Self>) {
        self.inner.idle_watch.notified().await;
    }

    /// Removes the long-lived sender and asks the manager to terminate.
    pub async fn shutdown(self: Arc<Self>) {
        let sender = {
            let mut slot = self.inner.command_tx.lock().await;
            slot.take()
        };
        if let Some(sender) = sender {
            let _ = sender.send(ManagerCommand::Shutdown).await;
        }
    }
}

impl Lease {
    /// Returns the capacity and arms one bounded idle-manager command.
    pub async fn release(self) {
        let should_release = {
            let mut state = self.inner.state.lock().await;
            if state.leased && state.generation == self.generation {
                state.leased = false;
                state.idle_generation = Some(self.generation);
                state.idle_expired = false;
                true
            } else {
                false
            }
        };

        if !should_release {
            return;
        }

        let wake_waiter = match self.inner.wake_policy {
            WakePolicy::Normal => true,
            #[cfg(feature = "fault-fixture")]
            WakePolicy::SuppressOneReleaseWake => false,
        };
        if wake_waiter {
            self.inner.waiter_wake.notify_one();
        }

        // This PoC observes the initial owner's idle lifetime. The waiter
        // return still performs the same state transition and wake protocol,
        // but does not create a second manager lifetime after the supervisor
        // has already begun shutdown.
        if self.generation == 1 {
            let sender = {
                let slot = self.inner.command_tx.lock().await;
                slot.as_ref().cloned()
            };
            if let Some(sender) = sender {
                let _ = sender
                    .send(ManagerCommand::ArmIdle {
                        generation: self.generation,
                    })
                    .await;
            }
        }
    }
}

impl LeaseManager {
    /// Processes bounded idle commands until shutdown.
    pub async fn run(mut self) {
        while let Some(command) = self.commands.recv().await {
            match command {
                ManagerCommand::ArmIdle { generation } => {
                    let expired =
                        time::timeout(self.inner.idle_timeout, self.inner.idle_wake.notified())
                            .await
                            .is_err();

                    if expired {
                        let mut state = self.inner.state.lock().await;
                        if !state.leased && state.idle_generation == Some(generation) {
                            state.idle_expired = true;
                        }
                    }
                    self.inner.idle_watch.notify_one();
                }
                ManagerCommand::Shutdown => break,
            }
        }
    }
}

/// Route A native capture: the same LeasePool protocol with only the normal
/// policy. Timer events are intentionally absent from public capture hooks.
#[allow(dead_code)]
#[laplace_sdk::verify(tasks, name = "nonvendored_lease_pool_control")]
fn nonvendored_lease_pool_control(tasks: &mut laplace_sdk::rt::TaskSet) {
    // Keep native capture on the signal-completion side of the timeout. The
    // private Route B scenario arms the short virtual timeout to explore the
    // expiration race; Route A has no public timer hook.
    let (pool, owner_lease, manager) =
        LeasePool::new(Duration::from_millis(100), WakePolicy::Normal);
    let manager_handle = tasks.spawn(manager.run());
    let watcher_handle = tasks.spawn(Arc::clone(&pool).watch_idle());

    let waiter_pool = Arc::clone(&pool);
    let waiter_handle = tasks.spawn(async move {
        let lease = waiter_pool.acquire().await;
        lease.release().await;
    });

    let owner_pool = Arc::clone(&pool);
    let owner_done = pool.owner_done_signal();
    let supervisor_done = Arc::clone(&owner_done);
    tasks.spawn(async move {
        owner_pool.wait_until_waiter_is_parked().await;
        owner_lease.release().await;
        waiter_handle.await;
        owner_done.notify_one();
    });

    tasks.spawn(async move {
        supervisor_done.notified().await;
        pool.shutdown().await;
        manager_handle.await;
        watcher_handle.await;
    });
}
