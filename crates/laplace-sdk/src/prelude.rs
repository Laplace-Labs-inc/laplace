//! Prelude for Laplace SDK — common types and macros.
//!
//! Import this module to get convenient access to the most commonly used
//! Tracked* primitives and verification macros:
//!
//! ```rust,ignore
//! use laplace_sdk::prelude::*;
//! ```

pub use crate::axiom_target;
pub use crate::laplace_tracked;
pub use crate::verify;

pub use crate::TrackedGuard;
pub use crate::TrackedMutex;
pub use crate::TrackedStdGuard;
pub use crate::TrackedStdMutex;

pub use crate::TrackedRwLock;
pub use crate::TrackedRwLockReadGuard;
pub use crate::TrackedRwLockWriteGuard;
pub use crate::TrackedStdRwLock;
pub use crate::TrackedStdRwLockReadGuard;
pub use crate::TrackedStdRwLockWriteGuard;

pub use crate::TrackedAtomicBool;
pub use crate::TrackedAtomicU32;
pub use crate::TrackedAtomicU64;
pub use crate::TrackedAtomicUsize;

pub use crate::TrackedSemaphore;
pub use crate::TrackedSemaphorePermit;
