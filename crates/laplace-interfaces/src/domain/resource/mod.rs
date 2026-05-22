// SPDX-License-Identifier: Apache-2.0
//! Resource domain contracts — types and traits for resource tracking and quota enforcement

pub mod traits;
pub mod types;

pub use traits::{ResourceGuard, ResourceTracker, ResourceUsage};
pub use types::{
    RequestResult, ResourceCapacity, ResourceError, ResourceId, ResourceType, ThreadId,
    ThreadStatus,
};
