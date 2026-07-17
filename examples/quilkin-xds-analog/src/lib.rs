// SPDX-License-Identifier: Apache-2.0
//! Customer-owned analog of quilkin's xDS control-plane stream fan-out.
//!
//! The model keeps one subscriber and one resource type. It composes the
//! request stream, per-client response stream, change broadcast, and shutdown
//! watch without copying Quilkin implementation code or modeling transport.

use laplace_sdk::rt::{broadcast, mpsc, watch};

const FINAL_VERSION: u64 = 3;

/// Normal protocol or the explicit lost-forward counterfactual.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigPolicy {
    /// Every committed version is forwarded to the subscribed client.
    Normal,
    /// Test-only inverse of the commit-to-per-client-forward discipline.
    #[cfg(feature = "fault-fixture")]
    SuppressLastForward,
}

impl ConfigPolicy {
    fn suppresses(self, version: u64) -> bool {
        #[cfg(feature = "fault-fixture")]
        if matches!(self, Self::SuppressLastForward) {
            return version == FINAL_VERSION;
        }
        let _ = (self, version);
        false
    }
}

/// Client-to-server subscription request carrying the bounded response stream.
pub struct SubscribeRequest {
    pub responses: mpsc::Sender<u64>,
}

/// Commits three versions to the server's change-broadcast plane.
pub async fn commit_three_changes(changes: broadcast::Sender<u64>) {
    for version in 1..=FINAL_VERSION {
        let _ = changes.send(version);
    }
}

/// Consumes one subscription and forwards the final committed version.
///
/// `Lagged` is a recoverable receive branch: the forwarder records no failure,
/// continues receiving, and forwards version three when it becomes available.
pub async fn run_server(
    mut requests: mpsc::UnboundedReceiver<SubscribeRequest>,
    mut changes: broadcast::Receiver<u64>,
    mut shutdown: watch::Receiver<bool>,
    policy: ConfigPolicy,
) {
    let Some(request) = requests.recv().await else {
        return;
    };

    let mut final_seen = false;
    while !final_seen {
        match changes.recv().await {
            Ok(version) => {
                final_seen = version == FINAL_VERSION;
                if !policy.suppresses(version) {
                    let _ = request.responses.send(version).await;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_missed)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        }
    }

    let _ = shutdown.changed().await;
}

/// Registers one subscriber and waits until the final version arrives.
pub async fn run_client(
    request_tx: mpsc::UnboundedSender<SubscribeRequest>,
    response_tx: mpsc::Sender<u64>,
    mut responses: mpsc::Receiver<u64>,
) {
    let _ = request_tx.send(SubscribeRequest {
        responses: response_tx,
    });

    while let Some(version) = responses.recv().await {
        if version == FINAL_VERSION {
            return;
        }
    }
}

/// Route A public capture. The four customer channel families remain in the
/// verify body so the macro rewrite and native capture are observable.
#[allow(dead_code)]
#[laplace_sdk::verify(tasks, name = "quilkin_xds_subscribe_change_broadcast")]
fn quilkin_xds_subscribe_change_broadcast(tasks: &mut laplace_sdk::rt::TaskSet) {
    #[allow(unused_imports)]
    use tokio::sync::{broadcast, mpsc, watch};

    let (request_tx, request_rx) = mpsc::unbounded_channel();
    let (response_tx, response_rx) = mpsc::channel(2);
    let (changes_tx, changes_rx) = broadcast::channel(2);
    let (shutdown_tx, shutdown_rx) = watch::channel(true);
    let retained_response_tx = response_tx.clone();

    let mutator = tasks.spawn(async move {
        commit_three_changes(changes_tx).await;
    });
    let server = tasks.spawn(async move {
        run_server(request_rx, changes_rx, shutdown_rx, ConfigPolicy::Normal).await;
    });
    let client = tasks.spawn(async move {
        run_client(request_tx, response_tx, response_rx).await;
    });
    let shutdown = tasks.spawn(async move {
        mutator.await;
        let _ = shutdown_tx.send(false);
    });
    tasks.spawn(async move {
        shutdown.await;
        server.await;
        let _retained_response_tx = retained_response_tx;
        client.await;
    });
}
