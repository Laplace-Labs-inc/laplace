//! # Tenant Domain Model
//!
//! Core domain entities representing tenant identity, subscription tiers, and resource quotas.
//!
//! ## Module Organization
//!
//! **`tier.rs`** - Tenant subscription tier classification
//! - `TenantTier`: Five-tier subscription model (Free, Standard, Turbo, Pro, Enterprise)
//! - Encapsulates business rules for tier progression and feature gating
//!
//! **`model.rs`** - Tenant metadata and resource configuration
//! - `ResourceConfig`: Resource quotas and execution limits for each tier
//! - `TenantMetadata`: Tenant domain entity with identity, tier, and configuration
//! - Maps tier definitions to concrete resource limits and monitoring settings
//!
//! ## Design Principles
//!
//! - **Tier-Driven Configuration**: All resource limits derive from tier classification
//! - **No Infrastructure Coupling**: Pure business data and logic, no adapter dependencies
//! - **Deterministic Mapping**: Every tenant configuration maps uniquely to tier capabilities
//! - **Single Source of Truth**: Tier quotas defined in one place, inherited by all consumers

pub mod model;
pub mod tier;

pub use model::{ResourceConfig, TenantMetadata};
pub use tier::TenantTier;
