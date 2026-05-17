// SPDX-License-Identifier: Apache-2.0
//! # Infrastructure Layer
//!
//! Low-level system abstractions and memory management primitives.
//!
//! The infrastructure layer contains foundational components that support
//! higher-level kernel and runtime functionality. This includes FFI boundaries,
//! memory layouts, and system-level abstractions that remain stable across
//! multiple releases.
//!
//! ## Module Organization
//!
//! - `abi`: Sovereign Bridge ABI v1.1.0 definitions
//!   - FFI-safe memory layouts for Rust-Deno boundary
//!   - Compile-time alignment verification
//!   - Thread safety guarantees for cross-boundary data transfer

pub mod abi;
