//! Sovereign Seed Primitives — re-exported from `laplace-interfaces`
//!
//! All substantive type definitions now live in `laplace_interfaces::domain::entropy::types`.
//! This module re-exports them so that existing consumers in laplace-core and laplace-kraken
//! continue to work without path changes.

pub use laplace_interfaces::domain::entropy::types::{
    ContextId, GlobalSeedConfig, LocalSeed, SeedAssignment,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entropy::{SeedDerive, SeedVerify};

    #[test]
    fn test_context_id_creation_and_display() {
        let ctx = ContextId::new(42);
        assert_eq!(ctx.as_u64(), 42);
        assert_eq!(ctx.to_string(), "Ctx#42");
    }

    #[test]
    fn test_local_seed_derive_deterministic() {
        let global = 12345u64;
        let ctx1 = ContextId::new(1);
        let ctx2 = ContextId::new(1);

        let seed1 = LocalSeed::derive(global, ctx1);
        let seed2 = LocalSeed::derive(global, ctx2);
        assert_eq!(seed1, seed2);
    }

    #[test]
    fn test_seed_assignment_verification() {
        let global = 12345u64;
        let ctx = ContextId::new(7);
        let local_seed = LocalSeed::derive(global, ctx);
        let assignment = SeedAssignment::new(ctx, local_seed, 1);

        assert!(assignment.verify(global));
    }

    #[test]
    fn test_global_seed_config() {
        let config = GlobalSeedConfig::new(9999, 16, 100);
        assert_eq!(config.seed, 9999);
        assert_eq!(config.lamport_mod, 16);
        assert_eq!(config.max_contexts, 100);
    }
}
