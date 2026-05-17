//! # Shared Memory Metadata
//!
//! Synchronization and coordination structures for turbo-mode shared memory
//! communication between Kernel and SDK processes.

use super::primitives::FfiLockState;
use super::FfiValidatable;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Shared Memory Metadata (for turbo-tier)
///
/// Stored at the head of turbo-mode shared memory region.
/// Enables Kernel and SDK to coordinate without syscalls through lock-free
/// synchronization primitives.
///
/// **Memory Layout (32 bytes, 8-byte aligned)**:
/// - Offset 0: `version` (u32, 4 bytes)
/// - Offset 4: `lock_state` (u32, 4 bytes)
/// - Offset 8: `kernel_ts` (u64, 8 bytes)
/// - Offset 16: `sdk_ts` (u64, 8 bytes)
/// - Offset 24: `data_offset` (u32, 4 bytes)
/// - Offset 28: `data_size` (u32, 4 bytes)
///
/// Total: 32 bytes (8-byte aligned)
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_ABI",
        link = "LEP-0003-laplace-interfaces-shared_memory_abi"
    )
)]
#[repr(C, align(8))]
#[derive(Debug, Clone)]
pub struct SharedMemoryMetadata {
    /// Version of shared memory format
    pub version: u32,

    /// Lock state (FfiLockState enum value: KERNEL_OWNED, SDK_OWNED, or LOCKED)
    pub lock_state: u32,

    /// Kernel's last write timestamp (nanoseconds since Unix epoch)
    pub kernel_ts: u64,

    /// SDK's last write timestamp (nanoseconds since Unix epoch)
    pub sdk_ts: u64,

    /// Offset to first data region (relative to metadata start)
    pub data_offset: u32,

    /// Total usable size after metadata
    pub data_size: u32,
}

impl SharedMemoryMetadata {
    /// Create new metadata structure
    pub const fn new(version: u32, data_offset: u32, data_size: u32) -> Self {
        Self {
            version,
            lock_state: FfiLockState::KERNEL_OWNED,
            kernel_ts: 0,
            sdk_ts: 0,
            data_offset,
            data_size,
        }
    }

    /// Verify metadata integrity
    pub fn is_valid(&self) -> bool {
        self.data_offset > 0 && self.data_size > 0 && self.version > 0
    }

    /// Check if lock is owned by kernel
    pub fn is_kernel_owned(&self) -> bool {
        self.lock_state == FfiLockState::KERNEL_OWNED
    }

    /// Check if lock is owned by SDK
    pub fn is_sdk_owned(&self) -> bool {
        self.lock_state == FfiLockState::SDK_OWNED
    }

    /// Check if lock is in contention
    pub fn is_locked(&self) -> bool {
        self.lock_state == FfiLockState::LOCKED
    }

    /// Update kernel timestamp (ns since Unix epoch)
    pub fn update_kernel_timestamp(&mut self, timestamp: u64) {
        self.kernel_ts = timestamp;
    }

    /// Update SDK timestamp (ns since Unix epoch)
    pub fn update_sdk_timestamp(&mut self, timestamp: u64) {
        self.sdk_ts = timestamp;
    }
}

impl Default for SharedMemoryMetadata {
    fn default() -> Self {
        Self::new(1, 32, 1024)
    }
}

impl FfiValidatable for SharedMemoryMetadata {
    fn is_valid(&self) -> bool {
        SharedMemoryMetadata::is_valid(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem;

    #[test]
    fn shared_memory_metadata_size() {
        assert_eq!(
            mem::size_of::<SharedMemoryMetadata>(),
            32,
            "SharedMemoryMetadata must be exactly 32 bytes"
        );
        assert_eq!(
            mem::align_of::<SharedMemoryMetadata>(),
            8,
            "SharedMemoryMetadata must be 8-byte aligned"
        );
    }

    #[test]
    fn shared_memory_metadata_creation() {
        let metadata = SharedMemoryMetadata::new(1, 32, 1024);

        assert_eq!(metadata.version, 1);
        assert_eq!(metadata.data_offset, 32);
        assert_eq!(metadata.data_size, 1024);
        assert_eq!(metadata.kernel_ts, 0);
        assert_eq!(metadata.sdk_ts, 0);
    }

    #[test]
    fn shared_memory_metadata_validation_valid() {
        let metadata = SharedMemoryMetadata::new(1, 32, 1024);
        assert!(metadata.is_valid());
    }

    #[test]
    fn shared_memory_metadata_validation_zero_version() {
        let metadata = SharedMemoryMetadata::new(0, 32, 1024);
        assert!(!metadata.is_valid(), "Zero version should be invalid");
    }

    #[test]
    fn shared_memory_metadata_validation_zero_offset() {
        let metadata = SharedMemoryMetadata::new(1, 0, 1024);
        assert!(!metadata.is_valid(), "Zero offset should be invalid");
    }

    #[test]
    fn shared_memory_metadata_validation_zero_size() {
        let metadata = SharedMemoryMetadata::new(1, 32, 0);
        assert!(!metadata.is_valid(), "Zero size should be invalid");
    }

    #[test]
    fn shared_memory_metadata_lock_states() {
        let metadata = SharedMemoryMetadata::new(1, 32, 1024);

        assert!(metadata.is_kernel_owned());
        assert!(!metadata.is_sdk_owned());
        assert!(!metadata.is_locked());
    }

    #[test]
    fn shared_memory_metadata_lock_transitions() {
        let mut metadata = SharedMemoryMetadata::new(1, 32, 1024);

        // Initial state: kernel owned
        assert!(metadata.is_kernel_owned());

        // Transition to SDK owned
        metadata.lock_state = FfiLockState::SDK_OWNED;
        assert!(metadata.is_sdk_owned());
        assert!(!metadata.is_kernel_owned());

        // Transition to locked
        metadata.lock_state = FfiLockState::LOCKED;
        assert!(metadata.is_locked());
    }

    #[test]
    fn shared_memory_metadata_timestamps() {
        let mut metadata = SharedMemoryMetadata::new(1, 32, 1024);

        assert_eq!(metadata.kernel_ts, 0);
        assert_eq!(metadata.sdk_ts, 0);

        metadata.update_kernel_timestamp(1000);
        assert_eq!(metadata.kernel_ts, 1000);

        metadata.update_sdk_timestamp(2000);
        assert_eq!(metadata.sdk_ts, 2000);
    }

    #[test]
    fn shared_memory_metadata_default_lock_state() {
        let metadata = SharedMemoryMetadata::new(1, 32, 1024);
        assert_eq!(metadata.lock_state, FfiLockState::KERNEL_OWNED);
    }

    #[test]
    fn shared_memory_metadata_default_constructor() {
        let metadata = SharedMemoryMetadata::default();
        assert!(metadata.is_valid());
        assert!(metadata.is_kernel_owned());
    }

    #[test]
    fn shared_memory_metadata_validatable_trait() {
        let metadata = SharedMemoryMetadata::new(1, 32, 1024);
        assert!(metadata.is_valid());
        // Trait method should agree with inherent method
        assert_eq!(
            SharedMemoryMetadata::is_valid(&metadata),
            <SharedMemoryMetadata as FfiValidatable>::is_valid(&metadata)
        );
    }
}
