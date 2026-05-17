//! Pool Management Policy
//!
//! Pure business logic for resource allocation, eviction, preemption, and storage routing.
//! This module contains zero infrastructure dependencies and serves as the decision layer
//! for adapters to implement.

// StorageStrategy is now defined in laplace-interfaces
pub use laplace_interfaces::domain::pool::StorageStrategy;

use laplace_interfaces::domain::TenantTier;
use std::time::{Duration, Instant};

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Pool Policy: Pure Decision Functions
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Pool management policy decisions
///
/// This structure encapsulates pure business logic for resource allocation,
/// eviction, preemption, and storage routing. All methods are stateless decision
/// functions that the adapters layer implements through actual resource management.
pub struct PoolPolicy;

impl PoolPolicy {
    /// Determine if an idle resource should be evicted
    ///
    /// Resources that remain idle beyond the configured threshold are candidates
    /// for eviction to free memory and reduce infrastructure costs while maintaining
    /// warm cache for active tenants.
    ///
    /// # Arguments
    ///
    /// * `last_used` - Instant when the resource was last accessed
    /// * `max_idle` - Maximum allowed idle duration
    ///
    /// # Returns
    ///
    /// `true` if the resource has exceeded the idle threshold and should be evicted
    pub fn should_evict(last_used: Instant, max_idle: Duration) -> bool {
        last_used.elapsed() > max_idle
    }

    /// Determine appropriate storage strategy for a tenant tier
    ///
    /// This decision function maps tenant tiers to storage strategies, establishing
    /// the fundamental performance contract for each tier. Higher tiers receive
    /// zero-copy acceleration while lower tiers use standard FFI.
    ///
    /// # Business Rule
    ///
    /// - Free and Standard tiers: Standard FFI with Protobuf serialization
    /// - Turbo, Pro, and Enterprise tiers: Turbo zero-copy with shared memory
    ///
    /// # Arguments
    ///
    /// * `tier` - Tenant subscription tier
    ///
    /// # Returns
    ///
    /// `StorageStrategy::Standard` for FFI-based storage
    /// `StorageStrategy::Turbo` for shared memory zero-copy storage
    pub fn determine_storage_strategy(tier: TenantTier) -> StorageStrategy {
        if tier.uses_turbo_acceleration() {
            StorageStrategy::Turbo
        } else {
            StorageStrategy::Standard
        }
    }

    /// Calculate resource allocation priority for a tier
    ///
    /// When resources are constrained, higher tiers receive preferential access
    /// to available capacity. This priority score guides preemption decisions and
    /// quota allocation.
    ///
    /// # Priority Scale
    ///
    /// The priority scores follow the tier hierarchy:
    /// - Free: 1 (lowest priority, highest preemption vulnerability)
    /// - Standard: 2
    /// - Turbo: 5
    /// - Pro: 8
    /// - Enterprise: 10 (highest priority, can preempt all lower tiers)
    ///
    /// # Arguments
    ///
    /// * `tier` - Tenant subscription tier
    ///
    /// # Returns
    ///
    /// Priority value where higher numbers indicate greater resource claim priority
    pub fn allocation_priority(tier: TenantTier) -> u8 {
        match tier {
            TenantTier::Free => 1,
            TenantTier::Standard => 2,
            TenantTier::Turbo => 5,
            TenantTier::Pro => 8,
            TenantTier::Enterprise => 10,
        }
    }

    /// Determine if one tenant can preempt another's resources
    ///
    /// Under resource scarcity, higher-tier tenants can forcibly reclaim resources
    /// from lower-tier tenants. This preemption hierarchy ensures that paying
    /// customers maintain service quality during congestion events.
    ///
    /// # Preemption Matrix
    ///
    /// A tenant can preempt another if its priority strictly exceeds the occupant's:
    ///
    /// | Requester     | Can Preempt          |
    /// |---------------|----------------------|
    /// | Free          | None                 |
    /// | Standard      | Free                 |
    /// | Turbo         | Free, Standard       |
    /// | Pro           | Free, Standard       |
    /// | Enterprise    | All lower tiers      |
    ///
    /// # Arguments
    ///
    /// * `requester` - Tier attempting to acquire resources
    /// * `occupant` - Tier currently holding resources
    ///
    /// # Returns
    ///
    /// `true` if the requester has sufficient priority to preempt the occupant
    pub fn can_preempt(requester: TenantTier, occupant: TenantTier) -> bool {
        let requester_priority = Self::allocation_priority(requester);
        let occupant_priority = Self::allocation_priority(occupant);

        // Preemption requires strictly higher priority
        requester_priority > occupant_priority
    }

    /// Select the best victim for preemption
    ///
    /// When multiple resources are available for preemption, this function identifies
    /// the optimal candidate based on priority and idle time. Preference is given to
    /// lower-priority tenants that have been idle longest, minimizing disruption.
    ///
    /// # Selection Criteria
    ///
    /// 1. Only consider tenants that can be preempted by the requester
    /// 2. Among eligible tenants, prefer lowest priority
    /// 3. Among same priority, prefer longest idle time
    ///
    /// # Arguments
    ///
    /// * `requester` - Tier attempting to acquire resources
    /// * `candidates` - List of (tier, last_used) tuples representing available victims
    ///
    /// # Returns
    ///
    /// Index of the best victim, or `None` if no tenant can be preempted
    pub fn select_preemption_victim(
        requester: TenantTier,
        candidates: &[(TenantTier, Instant)],
    ) -> Option<usize> {
        let mut best_victim: Option<(usize, u8, Duration)> = None;

        for (idx, (victim_tier, last_used)) in candidates.iter().enumerate() {
            if !Self::can_preempt(requester, *victim_tier) {
                continue;
            }

            let priority = Self::allocation_priority(*victim_tier);
            let idle_time = last_used.elapsed();

            match best_victim {
                None => {
                    best_victim = Some((idx, priority, idle_time));
                }
                Some((_, best_priority, best_idle)) => {
                    if priority < best_priority
                        || (priority == best_priority && idle_time > best_idle)
                    {
                        best_victim = Some((idx, priority, idle_time));
                    }
                }
            }
        }

        best_victim.map(|(idx, _, _)| idx)
    }

    /// Determine if a tenant's request rate should be throttled
    ///
    /// To prevent abuse and ensure fair resource distribution, free tiers have
    /// aggressive rate limiting while paid tiers have generous limits proportional
    /// to their tier value.
    ///
    /// # Rate Limits
    ///
    /// - Free: 60 requests per minute (1 req/sec)
    /// - Standard: 300 requests per minute (5 req/sec)
    /// - Turbo: 1200 requests per minute (20 req/sec)
    /// - Pro: 6000 requests per minute (100 req/sec)
    /// - Enterprise: Unlimited
    ///
    /// # Arguments
    ///
    /// * `tier` - Tenant subscription tier
    /// * `requests_per_minute` - Current request rate
    ///
    /// # Returns
    ///
    /// `true` if the request rate exceeds the tier's limit and should be throttled
    pub fn should_throttle(tier: TenantTier, requests_per_minute: u64) -> bool {
        let limit = match tier {
            TenantTier::Free => 60,
            TenantTier::Standard => 300,
            TenantTier::Turbo => 1200,
            TenantTier::Pro => 6000,
            TenantTier::Enterprise => u64::MAX,
        };

        requests_per_minute > limit
    }

    /// Calculate LRU eviction threshold for a tier
    ///
    /// Higher tiers receive longer cache retention to provide consistent performance
    /// during periods of inactivity. Free tiers are evicted aggressively to minimize
    /// memory utilization.
    ///
    /// # Retention Periods
    ///
    /// - Free: 1 minute
    /// - Standard: 5 minutes
    /// - Turbo: 10 minutes
    /// - Pro: 30 minutes
    /// - Enterprise: 1 hour
    ///
    /// # Arguments
    ///
    /// * `tier` - Tenant subscription tier
    ///
    /// # Returns
    ///
    /// Maximum idle duration before the resource becomes eligible for eviction
    pub fn eviction_threshold(tier: TenantTier) -> Duration {
        match tier {
            TenantTier::Free => Duration::from_secs(60),
            TenantTier::Standard => Duration::from_secs(300),
            TenantTier::Turbo => Duration::from_secs(600),
            TenantTier::Pro => Duration::from_secs(1800),
            TenantTier::Enterprise => Duration::from_secs(3600),
        }
    }

    /// Calculate recommended pool capacity for a tier
    ///
    /// Pool sizing follows tier requirements for concurrent request handling.
    /// Higher tiers receive larger pools to accommodate burst traffic patterns
    /// without exceeding capacity limits.
    ///
    /// # Pool Sizes
    ///
    /// - Free: 10 slots
    /// - Standard: 50 slots
    /// - Turbo: 200 slots
    /// - Pro: 500 slots
    /// - Enterprise: 1000 slots
    ///
    /// # Arguments
    ///
    /// * `tier` - Tenant subscription tier
    ///
    /// # Returns
    ///
    /// Recommended pool capacity for this tier
    pub fn recommended_pool_size(tier: TenantTier) -> usize {
        match tier {
            TenantTier::Free => 10,
            TenantTier::Standard => 50,
            TenantTier::Turbo => 200,
            TenantTier::Pro => 500,
            TenantTier::Enterprise => 1000,
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_evict() {
        let now = Instant::now();
        let max_idle = Duration::from_secs(300);

        assert!(!PoolPolicy::should_evict(now, max_idle));

        let old_time = now - Duration::from_secs(400);
        assert!(PoolPolicy::should_evict(old_time, max_idle));
    }

    #[test]
    fn test_storage_strategy_determination() {
        assert_eq!(
            PoolPolicy::determine_storage_strategy(TenantTier::Free),
            StorageStrategy::Standard
        );
        assert_eq!(
            PoolPolicy::determine_storage_strategy(TenantTier::Standard),
            StorageStrategy::Standard
        );
        assert_eq!(
            PoolPolicy::determine_storage_strategy(TenantTier::Turbo),
            StorageStrategy::Turbo
        );
        assert_eq!(
            PoolPolicy::determine_storage_strategy(TenantTier::Pro),
            StorageStrategy::Turbo
        );
        assert_eq!(
            PoolPolicy::determine_storage_strategy(TenantTier::Enterprise),
            StorageStrategy::Turbo
        );
    }

    #[test]
    fn test_allocation_priority() {
        assert_eq!(PoolPolicy::allocation_priority(TenantTier::Free), 1);
        assert_eq!(PoolPolicy::allocation_priority(TenantTier::Standard), 2);
        assert_eq!(PoolPolicy::allocation_priority(TenantTier::Turbo), 5);
        assert_eq!(PoolPolicy::allocation_priority(TenantTier::Pro), 8);
        assert_eq!(PoolPolicy::allocation_priority(TenantTier::Enterprise), 10);
    }

    #[test]
    fn test_preemption_rules() {
        assert!(PoolPolicy::can_preempt(
            TenantTier::Enterprise,
            TenantTier::Free
        ));
        assert!(PoolPolicy::can_preempt(
            TenantTier::Enterprise,
            TenantTier::Standard
        ));
        assert!(PoolPolicy::can_preempt(
            TenantTier::Enterprise,
            TenantTier::Turbo
        ));
        assert!(PoolPolicy::can_preempt(
            TenantTier::Enterprise,
            TenantTier::Pro
        ));

        assert!(PoolPolicy::can_preempt(TenantTier::Turbo, TenantTier::Free));
        assert!(PoolPolicy::can_preempt(
            TenantTier::Turbo,
            TenantTier::Standard
        ));
        assert!(!PoolPolicy::can_preempt(TenantTier::Turbo, TenantTier::Pro));

        assert!(!PoolPolicy::can_preempt(
            TenantTier::Free,
            TenantTier::Standard
        ));
        assert!(!PoolPolicy::can_preempt(
            TenantTier::Free,
            TenantTier::Turbo
        ));

        assert!(!PoolPolicy::can_preempt(
            TenantTier::Standard,
            TenantTier::Standard
        ));
        assert!(!PoolPolicy::can_preempt(
            TenantTier::Turbo,
            TenantTier::Turbo
        ));
    }

    #[test]
    fn test_select_preemption_victim() {
        let now = Instant::now();
        let old = now - Duration::from_secs(100);
        let very_old = now - Duration::from_secs(500);

        let candidates = vec![
            (TenantTier::Standard, now),
            (TenantTier::Free, old),
            (TenantTier::Free, very_old),
            (TenantTier::Turbo, old),
        ];

        let victim = PoolPolicy::select_preemption_victim(TenantTier::Enterprise, &candidates);
        assert_eq!(victim, Some(2));

        let victim = PoolPolicy::select_preemption_victim(TenantTier::Turbo, &candidates);
        assert_eq!(victim, Some(2));

        let victim = PoolPolicy::select_preemption_victim(TenantTier::Free, &candidates);
        assert_eq!(victim, None);
    }

    #[test]
    fn test_throttling() {
        assert!(PoolPolicy::should_throttle(TenantTier::Free, 61));
        assert!(!PoolPolicy::should_throttle(TenantTier::Free, 59));

        assert!(!PoolPolicy::should_throttle(TenantTier::Turbo, 1000));
        assert!(PoolPolicy::should_throttle(TenantTier::Turbo, 1300));

        assert!(!PoolPolicy::should_throttle(
            TenantTier::Enterprise,
            1_000_000
        ));
    }

    #[test]
    fn test_eviction_thresholds() {
        assert_eq!(
            PoolPolicy::eviction_threshold(TenantTier::Free),
            Duration::from_secs(60)
        );
        assert_eq!(
            PoolPolicy::eviction_threshold(TenantTier::Turbo),
            Duration::from_secs(600)
        );
        assert_eq!(
            PoolPolicy::eviction_threshold(TenantTier::Enterprise),
            Duration::from_secs(3600)
        );
    }

    #[test]
    fn test_recommended_pool_sizes() {
        assert_eq!(PoolPolicy::recommended_pool_size(TenantTier::Free), 10);
        assert_eq!(PoolPolicy::recommended_pool_size(TenantTier::Standard), 50);
        assert_eq!(PoolPolicy::recommended_pool_size(TenantTier::Turbo), 200);
        assert_eq!(PoolPolicy::recommended_pool_size(TenantTier::Pro), 500);
        assert_eq!(
            PoolPolicy::recommended_pool_size(TenantTier::Enterprise),
            1000
        );
    }

    #[test]
    fn test_storage_strategy_properties() {
        assert_eq!(StorageStrategy::Standard.expected_latency_ns(), 41_500);
        assert_eq!(StorageStrategy::Turbo.expected_latency_ns(), 500);

        assert!(!StorageStrategy::Standard.is_zero_copy());
        assert!(StorageStrategy::Turbo.is_zero_copy());

        assert_eq!(StorageStrategy::Standard.name(), "Standard-FFI");
        assert_eq!(StorageStrategy::Turbo.name(), "Turbo-ZeroCopy");
    }

    #[test]
    fn test_storage_strategy_display() {
        assert_eq!(format!("{}", StorageStrategy::Standard), "Standard-FFI");
        assert_eq!(format!("{}", StorageStrategy::Turbo), "Turbo-ZeroCopy");
    }
}
