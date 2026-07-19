// SPDX-License-Identifier: Apache-2.0
//! Customer-owned analog of quilkin's FilterChain store/notify/reload contract.
//!
//! This is a contract re-expression, not copied quilkin code. The state plane
//! uses the evidence-only ModelArcSwap surface and the notification plane uses
//! the real tokio broadcast channel through Laplace's wrapper.

use std::sync::Arc;

use laplace_sdk::rt::{broadcast, ModelArcSwap};

/// Policy used to select the normal protocol or the explicit fault fixture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReloadPolicy {
    /// Every store publishes the reload notification.
    Normal,
    /// Test-only inverse of the store-to-notify discipline.
    #[cfg(feature = "fault-fixture")]
    SuppressOneReloadWake,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FilterChainConfig {
    revision: u64,
    filters: Vec<&'static str>,
    shutdown: bool,
}

/// A bounded FilterChain state/notification pair.
pub struct FilterChain {
    config: ModelArcSwap<FilterChainConfig>,
    notify: broadcast::Sender<()>,
    policy: ReloadPolicy,
}

impl FilterChain {
    /// Creates the initial empty chain at revision zero.
    #[must_use]
    pub fn new(notify: broadcast::Sender<()>, policy: ReloadPolicy) -> Arc<Self> {
        Arc::new(Self {
            config: ModelArcSwap::new(FilterChainConfig {
                revision: 0,
                filters: Vec::new(),
                shutdown: false,
            }),
            notify,
            policy,
        })
    }

    /// Stores a complete configuration, then publishes the reload event.
    pub async fn reload_to(&self, filters: Vec<&'static str>) {
        self.config.store(Arc::new(FilterChainConfig {
            revision: 1,
            filters,
            shutdown: true,
        }));

        let should_notify = match self.policy {
            ReloadPolicy::Normal => true,
            #[cfg(feature = "fault-fixture")]
            ReloadPolicy::SuppressOneReloadWake => false,
        };
        if should_notify {
            let _ = self.notify.send(());
        }
    }

    /// Waits for broadcast notifications and reloads one consistent snapshot.
    pub async fn consumer_loop(&self, mut receiver: broadcast::Receiver<()>) {
        let mut last_revision = 0;
        loop {
            match receiver.recv().await {
                Ok(()) => {
                    let snapshot = self.config.load_full();
                    if snapshot.revision > last_revision {
                        last_revision = snapshot.revision;
                        let _filter_count = snapshot.filters.len();
                        if snapshot.shutdown {
                            return;
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    }
}

/// Route A public capture. The `tokio::sync::broadcast` import and channel
/// constructor in this function are intentionally left in customer syntax so
/// `#[laplace_sdk::verify(tasks)]` proves the macro rewrite path. ArcSwap is
/// adopted explicitly through `laplace_model_rt::ModelArcSwap`; it is not a macro
/// rewrite surface.
#[allow(dead_code)]
#[laplace_sdk::verify(tasks, name = "quilkin_filter_chain_store_notify_reload")]
fn quilkin_filter_chain_store_notify_reload(tasks: &mut laplace_sdk::rt::TaskSet) {
    // The model rewrite consumes this alias while leaving the source import
    // intact; allow the post-rewrite import warning so the customer spelling
    // remains visible in the fixture.
    #[allow(unused_imports)]
    use tokio::sync::broadcast;

    let (notify, receiver) = broadcast::channel(8);
    let chain = FilterChain::new(notify, ReloadPolicy::Normal);
    let mutator_chain = Arc::clone(&chain);
    let mutator = tasks.spawn(async move {
        mutator_chain.reload_to(vec!["geoip", "rate_limit"]).await;
    });

    let consumer_chain = Arc::clone(&chain);
    let consumer = tasks.spawn(async move {
        consumer_chain.consumer_loop(receiver).await;
    });

    tasks.spawn(async move {
        mutator.await;
        consumer.await;
    });
}
