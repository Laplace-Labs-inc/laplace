//! # Primitive ABI Types
//!
//! Core FFI structures for Rust-Deno zero-copy bridge.
//! Defines lock states, buffers, and response wrappers with guaranteed memory layout.

use super::FfiValidatable;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Shared Memory Lock States
///
/// Used for synchronization between Kernel and KNUL/SDK processes.
/// Three distinct states ensure efficient lock-free coordination:
/// - KERNEL_OWNED: Kernel holds exclusive access (reader phase)
/// - SDK_OWNED: SDK holds exclusive access (writer phase)
/// - LOCKED: Contention detected; fallback to mutual exclusion
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Primitives",
        link = "LEP-0002-laplace-interfaces-ffi_primitives"
    )
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FfiLockState;

impl FfiLockState {
    /// Lock owned by Kernel (reader phase)
    pub const KERNEL_OWNED: u32 = 0;
    /// Lock owned by SDK/KNUL (writer phase)
    pub const SDK_OWNED: u32 = 1;
    /// Lock in contention state
    pub const LOCKED: u32 = 2;
}

// ============================================================================
// FFI Buffer (32 bytes)
// ============================================================================

/// FFI-safe buffer for zero-copy data transfer
///
/// Represents a contiguous memory region with explicit padding for 64-bit alignment.
///
/// **Memory Layout (32 bytes, 8-byte aligned)**:
/// - Offset 0: `data` (pointer, 8 bytes)
/// - Offset 8: `len` (usize, 8 bytes on 64-bit)
/// - Offset 16: `cap` (usize, 8 bytes on 64-bit)
/// - Offset 24: `_padding` (u64, 8 bytes)
///
/// Total: 32 bytes (8-byte aligned)
///
/// # Safety
///
/// FfiBuffer is safe to send and share between threads because:
/// 1. The pointer always references kernel-managed memory (never user-allocated)
/// 2. Ownership is never transferred; FfiBuffer is always used behind Arc/DashMap
/// 3. The kernel guarantees memory stability while server is running
/// 4. Access is synchronized via Arc/DashMap/AtomicU64 in containing structures
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Primitives",
        link = "LEP-0002-laplace-interfaces-ffi_primitives"
    )
)]
#[repr(C, align(8))]
#[derive(Debug, Clone)]
pub struct FfiBuffer {
    /// Pointer to buffer data (must be pinned in kernel-managed memory)
    pub data: *mut u8,
    /// Current length (valid data bytes)
    pub len: usize,
    /// Capacity (total allocated bytes)
    pub cap: usize,
    /// Explicit padding for alignment; reserved for future lock metadata
    pub _padding: u64,
}

impl FfiBuffer {
    /// Create a new empty FfiBuffer
    pub const fn new() -> Self {
        Self {
            data: std::ptr::null_mut(),
            len: 0,
            cap: 0,
            _padding: 0,
        }
    }

    /// Verify buffer is valid for FFI crossing
    pub fn is_valid(&self) -> bool {
        // Null ptr with zero len/cap is valid (empty buffer)
        if self.data.is_null() {
            return self.len == 0 && self.cap == 0;
        }
        // Non-null buffer must have cap >= len
        self.cap >= self.len && self.len > 0
    }
}

impl Default for FfiBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl FfiValidatable for FfiBuffer {
    fn is_valid(&self) -> bool {
        FfiBuffer::is_valid(self)
    }
}

// ============================================================================
// THREAD SAFETY IMPLEMENTATIONS
// ============================================================================

/// FfiBuffer is safe to send between threads
///
/// Although FfiBuffer contains a raw pointer (*mut u8), it is safe to send
/// between threads because:
/// 1. The pointer always references kernel-managed memory (never user-allocated)
/// 2. Ownership is never transferred; FfiBuffer is always used behind Arc/DashMap
/// 3. The kernel guarantees memory stability while server is running
/// 4. Access is synchronized via Arc/DashMap/AtomicU64 in containing structures
///
/// # Safety Justification
/// - Kernel owns all memory referenced by FfiBuffer
/// - No race conditions: memory access patterns are coordinated via FFI boundary
/// - Pointer validity guaranteed by ownership model
unsafe impl Send for FfiBuffer {}

/// FfiBuffer is safe to share between threads
///
/// See Send implementation above for detailed justification.
/// Sync is safe because FfiBuffer is immutable after creation (fields are pub but not mut).
unsafe impl Sync for FfiBuffer {}

// ============================================================================
// FFI Response (40 bytes)
// ============================================================================

/// FFI Response wrapper for Kernel ↔ SDK communication
///
/// Encapsulates result or error from FFI operations, with result payload
/// stored in an FfiBuffer for zero-copy transfer.
///
/// **Memory Layout (40 bytes, 8-byte aligned)**:
/// - Offset 0: `error_code` (u32, 4 bytes)
/// - Offset 4: `_reserved` (u32, 4 bytes)
/// - Offset 8: `result_buffer` (FfiBuffer, 32 bytes)
///
/// Total: 40 bytes (aligned to 8 bytes)
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Primitives",
        link = "LEP-0002-laplace-interfaces-ffi_primitives"
    )
)]
#[repr(C, align(8))]
#[derive(Debug, Clone)]
pub struct FfiResponse {
    /// Error code (0 = SUCCESS)
    pub error_code: u32,
    /// Reserved for future use (alignment padding)
    pub _reserved: u32,
    /// Result payload (if error_code == 0)
    pub result_buffer: FfiBuffer,
}

impl FfiResponse {
    /// Create a successful response with buffer
    pub const fn success(buffer: FfiBuffer) -> Self {
        Self {
            error_code: 0,
            _reserved: 0,
            result_buffer: buffer,
        }
    }

    /// Create an error response with error code
    pub const fn error(error_code: u32) -> Self {
        Self {
            error_code,
            _reserved: 0,
            result_buffer: FfiBuffer::new(),
        }
    }

    /// Check if response indicates success
    pub fn is_success(&self) -> bool {
        self.error_code == 0
    }

    /// Check if response indicates error
    pub fn is_error(&self) -> bool {
        self.error_code != 0
    }
}

impl Default for FfiResponse {
    fn default() -> Self {
        Self::error(1000) // LaplaceError::Internal
    }
}

impl FfiValidatable for FfiResponse {
    fn is_valid(&self) -> bool {
        // Success responses must have valid result_buffer
        // Error responses must have non-zero error_code
        if self.is_success() {
            self.result_buffer.is_valid()
        } else {
            self.error_code != 0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffi_buffer_creation() {
        let buf = FfiBuffer::new();
        assert!(buf.data.is_null());
        assert_eq!(buf.len, 0);
        assert_eq!(buf.cap, 0);
        assert!(buf.is_valid());
    }

    #[test]
    fn ffi_buffer_size_and_align() {
        assert_eq!(
            std::mem::size_of::<FfiBuffer>(),
            32,
            "FfiBuffer must be exactly 32 bytes"
        );
        assert_eq!(
            std::mem::align_of::<FfiBuffer>(),
            8,
            "FfiBuffer must be 8-byte aligned"
        );
    }

    #[test]
    fn ffi_buffer_validatable_trait() {
        let buf = FfiBuffer::new();
        assert!(buf.is_valid());
        // Trait method should agree with inherent method
        assert_eq!(
            FfiBuffer::is_valid(&buf),
            <FfiBuffer as FfiValidatable>::is_valid(&buf)
        );
    }

    #[test]
    fn ffi_response_creation() {
        let buf = FfiBuffer::new();
        let resp = FfiResponse::success(buf);
        assert!(resp.is_success());
        assert!(!resp.is_error());
    }

    #[test]
    fn ffi_response_error() {
        let resp = FfiResponse::error(1000);
        assert!(!resp.is_success());
        assert!(resp.is_error());
        assert_eq!(resp.error_code, 1000);
    }

    #[test]
    fn ffi_response_size_and_align() {
        assert_eq!(
            std::mem::size_of::<FfiResponse>(),
            40,
            "FfiResponse must be exactly 40 bytes"
        );
        assert_eq!(
            std::mem::align_of::<FfiResponse>(),
            8,
            "FfiResponse must be 8-byte aligned"
        );
    }

    #[test]
    fn ffi_response_validatable_trait() {
        let resp = FfiResponse::error(1000);
        assert!(resp.is_valid());
        let resp_success = FfiResponse::success(FfiBuffer::new());
        assert!(resp_success.is_valid());
    }

    #[test]
    fn lock_state_constants() {
        assert_eq!(FfiLockState::KERNEL_OWNED, 0);
        assert_eq!(FfiLockState::SDK_OWNED, 1);
        assert_eq!(FfiLockState::LOCKED, 2);
    }
}
