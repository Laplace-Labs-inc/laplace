// SPDX-License-Identifier: Apache-2.0
//! The identities every contract operation is expressed in terms of.
//!
//! These moved here from `laplace-interfaces::domain::resource::types` unchanged
//! — same shape, same derives, same `laplace_meta` layer tag — because the
//! contract cannot be stated without them. `laplace-interfaces` re-exports them
//! at their original paths, so callers that name
//! `laplace_interfaces::domain::ThreadId` keep compiling.

use std::fmt;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Thread identifier (maps to TLA+ Threads)
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Resource",
        link = "LEP-0004-laplace-interfaces-resource_domain_contracts"
    )
)]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ThreadId(pub usize);

impl ThreadId {
    /// Creates a `ThreadId` from a raw index.
    ///
    /// - **Arguments:** `id` — zero-based thread index within the tracker.
    /// - **Returns:** A new `ThreadId` wrapping `id`.
    /// - **Ownership:** `id` is copied (primitive).
    #[inline(always)]
    pub const fn new(id: usize) -> Self {
        Self(id)
    }

    /// Returns the inner `usize` index.
    ///
    /// - **Returns:** The raw thread index originally passed to [`ThreadId::new`].
    /// - **Ownership:** `self` is copied (cheap `Copy` type).
    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

impl fmt::Display for ThreadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "t{}", self.0)
    }
}

/// Resource identifier (maps to TLA+ Resources)
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Resource",
        link = "LEP-0004-laplace-interfaces-resource_domain_contracts"
    )
)]
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ResourceId(pub usize);

impl ResourceId {
    /// Creates a `ResourceId` from a raw index.
    ///
    /// - **Arguments:** `id` — zero-based resource index within the tracker.
    /// - **Returns:** A new `ResourceId` wrapping `id`.
    /// - **Ownership:** `id` is copied (primitive).
    #[inline(always)]
    pub const fn new(id: usize) -> Self {
        Self(id)
    }

    /// Returns the inner `usize` index.
    ///
    /// - **Returns:** The raw resource index originally passed to [`ResourceId::new`].
    /// - **Ownership:** `self` is copied (cheap `Copy` type).
    #[inline(always)]
    pub const fn as_usize(self) -> usize {
        self.0
    }
}

impl fmt::Display for ResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r{}", self.0)
    }
}
