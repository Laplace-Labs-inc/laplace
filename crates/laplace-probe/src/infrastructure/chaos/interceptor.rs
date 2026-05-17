//! ChaosInterceptor — deterministic network chaos injection
//!
//! Implements [`PacketInterceptor`] by consulting a [`ChaosSchedule`] and a
//! [`NetworkClockProvider`] to decide whether each packet should be dropped
//! (network partition) or delayed (latency spike).

use std::sync::Arc;

use laplace_interfaces::domain::kraken::types::ChaosSchedule;
use laplace_interfaces::domain::transport::pluggable::{
    InterceptReason, NetworkClockProvider, PacketBuffer, PacketInterceptor,
};
use laplace_interfaces::TransportPacket;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Deterministic chaos interceptor driven by a [`ChaosSchedule`].
///
/// Plugged into the QUIC transport pipeline via the [`PacketInterceptor`] trait.
/// At each packet boundary it reads the current virtual time from the injected
/// clock and queries the schedule to decide:
///
/// 1. **Partition** — if `schedule.is_partitioned(time, vu_id)` the packet is
///    dropped with [`InterceptReason::NetworkPartition`].
/// 2. **Latency spike** — if `schedule.total_extra_latency_ms(time) > 0` the
///    corresponding delay (in microseconds) is returned from [`on_send`] so the
///    caller can sleep before transmitting.
pub struct ChaosInterceptor {
    /// Deterministic chaos script (shared across all connections)
    schedule: Arc<ChaosSchedule>,
    /// Virtual / deterministic clock for time queries
    clock: Arc<dyn NetworkClockProvider>,
    /// Virtual User ID used to evaluate per-VU chaos events (e.g. NetworkPartition ranges).
    ///
    /// Set to the specific VU's identifier when the interceptor is created per-VU,
    /// or `0` for server-level interceptors where VU identity is not yet available.
    vu_id: u64,
}

impl ChaosInterceptor {
    /// Create a new `ChaosInterceptor`.
    ///
    /// `vu_id` is used to evaluate `NetworkPartition` target ranges. Pass the
    /// actual VU identifier for per-VU interceptors, or `0` for server-level
    /// instances where VU identity is not yet resolved.
    pub fn new(
        schedule: Arc<ChaosSchedule>,
        clock: Arc<dyn NetworkClockProvider>,
        vu_id: u64,
    ) -> Self {
        Self {
            schedule,
            clock,
            vu_id,
        }
    }

    /// Current virtual time in milliseconds.
    #[inline]
    fn now_ms(&self) -> u64 {
        self.clock.now_us() / 1000
    }
}

impl PacketInterceptor for ChaosInterceptor {
    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(
            layer = "40_Probe_FFI",
            link = "LEP-0016-laplace-probe-ffi_barrier_and_deterministic_chaos"
        )
    )]
    fn on_receive(&self, _packet: &mut PacketBuffer) -> Result<(), InterceptReason> {
        let now_ms = self.now_ms();

        if self.schedule.is_partitioned(now_ms, self.vu_id) {
            return Err(InterceptReason::NetworkPartition);
        }
        Ok(())
    }

    fn on_send(&self, _packet: &TransportPacket) -> u64 {
        let now_ms = self.now_ms();
        let extra_ms = self.schedule.total_extra_latency_ms(now_ms);
        // Convert ms → μs for the caller
        extra_ms * 1000
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use laplace_interfaces::domain::kraken::types::ChaosEvent;
    use laplace_interfaces::domain::transport::pluggable::InterceptReason;

    /// Deterministic clock returning a fixed value for testing.
    struct FixedClock(u64);

    impl NetworkClockProvider for FixedClock {
        fn now_us(&self) -> u64 {
            self.0
        }
    }

    fn make_packet() -> TransportPacket {
        TransportPacket::new(vec![1, 2, 3], 1)
    }

    #[tokio::test]
    async fn latency_spike_injects_delay() {
        let mut schedule = ChaosSchedule::new();
        schedule.add_event(ChaosEvent::LatencySpike {
            start_ms: 100,
            end_ms: 200,
            extra_latency_ms: 50,
        });

        // Clock at 150ms (= 150_000 μs) — inside the spike window
        let interceptor =
            ChaosInterceptor::new(Arc::new(schedule), Arc::new(FixedClock(150_000)), 0);

        let packet = make_packet();
        let delay_us = interceptor.on_send(&packet);
        assert_eq!(delay_us, 50_000, "should return 50ms in μs");
    }

    #[tokio::test]
    async fn latency_spike_outside_window_returns_zero() {
        let mut schedule = ChaosSchedule::new();
        schedule.add_event(ChaosEvent::LatencySpike {
            start_ms: 100,
            end_ms: 200,
            extra_latency_ms: 50,
        });

        // Clock at 250ms — outside the spike window
        let interceptor =
            ChaosInterceptor::new(Arc::new(schedule), Arc::new(FixedClock(250_000)), 0);

        let packet = make_packet();
        let delay_us = interceptor.on_send(&packet);
        assert_eq!(delay_us, 0, "no delay outside the spike window");
    }

    #[tokio::test]
    async fn network_partition_drops_packet() {
        let mut schedule = ChaosSchedule::new();
        schedule.add_event(ChaosEvent::NetworkPartition {
            start_ms: 100,
            end_ms: 200,
            target_vu_range: 0..5,
        });

        // Clock at 150ms — inside the partition window; vu_id=0 is in 0..5
        let interceptor =
            ChaosInterceptor::new(Arc::new(schedule), Arc::new(FixedClock(150_000)), 0);

        let mut packet = make_packet();
        let result = interceptor.on_receive(&mut packet);
        assert_eq!(
            result,
            Err(InterceptReason::NetworkPartition),
            "packet should be dropped during partition"
        );
    }

    #[tokio::test]
    async fn network_partition_outside_window_passes() {
        let mut schedule = ChaosSchedule::new();
        schedule.add_event(ChaosEvent::NetworkPartition {
            start_ms: 100,
            end_ms: 200,
            target_vu_range: 0..5,
        });

        // Clock at 50ms — before the partition window
        let interceptor =
            ChaosInterceptor::new(Arc::new(schedule), Arc::new(FixedClock(50_000)), 0);

        let mut packet = make_packet();
        let result = interceptor.on_receive(&mut packet);
        assert_eq!(
            result,
            Ok(()),
            "packet should pass outside partition window"
        );
    }

    #[tokio::test]
    async fn combined_spike_and_partition() {
        let mut schedule = ChaosSchedule::new();
        schedule.add_event(ChaosEvent::LatencySpike {
            start_ms: 100,
            end_ms: 300,
            extra_latency_ms: 25,
        });
        schedule.add_event(ChaosEvent::NetworkPartition {
            start_ms: 150,
            end_ms: 250,
            target_vu_range: 0..3,
        });

        // Clock at 200ms — both events active; vu_id=0 is in 0..3
        let interceptor =
            ChaosInterceptor::new(Arc::new(schedule), Arc::new(FixedClock(200_000)), 0);

        // on_receive: partition should drop
        let mut packet = make_packet();
        assert_eq!(
            interceptor.on_receive(&mut packet),
            Err(InterceptReason::NetworkPartition),
        );

        // on_send: latency spike should still report delay
        let packet = make_packet();
        assert_eq!(interceptor.on_send(&packet), 25_000);
    }
}
