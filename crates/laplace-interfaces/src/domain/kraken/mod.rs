//! # Kraken Domain Contracts
//!
//! Interface definitions for the Kraken load-testing and digital-twin DSL.
//! All items in this module require the `twin` feature flag.
//!
//! ## Contents
//!
//! - **`types`** — data types for the scenario DSL, chaos scheduling, and load profiles

#![cfg(feature = "twin")]

pub mod types;

pub use types::{
    ChaosEvent, ChaosSchedule, RampUpProfile, Scenario, ScenarioStep, ThinkTimeDistribution,
    VUState,
};
