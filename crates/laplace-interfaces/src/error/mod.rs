//! # Error System Architecture
//!
//! Unified error handling for the Laplace platform with layered error taxonomy.
//!
//! ## Error Layers
//!
//! **Layer 1: FFI Boundary Codes** (`codes.rs`)
//! - `LaplaceError`: The authoritative enumeration for cross-language FFI communication.
//! - FFI-compatible (`#[repr(u32)]`), Protobuf-serializable, and TypeScript-codegen-ready.
//! - All error codes organized by severity and domain (0-6999 with semantic ranges).
//! - Single Source of Truth for all system error codes.
//!
//! **Layer 2: Domain-Specific Errors** (`tenant.rs`)
//! - `TenantError`: Business logic violations and operational failures at the tenant level.
//! - Maps to FFI boundary codes via `to_proto_code()` for SDK propagation.
//! - Implements domain-specific predicates (`is_recoverable()`, `suggests_upgrade()`).
//! - Carries rich context (tenant ID, execution time, resource snapshots).
//!
//! ## Design Principles
//!
//! - **Layered Architecture**: Domain errors are built atop FFI codes, not alongside.
//! - **Deterministic Mapping**: Every `TenantError` variant has a deterministic mapping
//!   to a `LaplaceError` code via `to_proto_code()`.
//! - **Forward Compatibility**: Unknown error codes map safely to `Internal` (1000).
//! - **Client-Side Intelligence**: Error predicates enable SDK to make decisions
//!   (retry, backoff, escalate, upgrade) without application-level logic.

pub mod codes;
#[cfg(feature = "twin")]
pub mod kraken;
pub mod tenant;
pub mod transport;

pub use codes::LaplaceError;
pub use tenant::TenantError;
pub use transport::TransportError;

#[cfg(feature = "twin")]
pub use kraken::{KrakenError, Result as KrakenResult};

/// Laplace Result type for convenience
///
/// Standard Result type wrapper using LaplaceError as the error variant.
/// Provides ergonomic error handling across the Laplace stack.
pub type LaplaceResult<T> = Result<T, crate::error::codes::LaplaceError>;
