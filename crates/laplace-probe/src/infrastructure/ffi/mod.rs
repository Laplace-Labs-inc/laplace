// SPDX-License-Identifier: Apache-2.0
//! FFI Utilities
//!
//! Low-level utilities for converting between FFI types and Rust representations.
//! Provides safe wrappers around raw pointer dereferences and buffer conversions.

use laplace_interfaces::FfiBuffer;
use std::slice;

/// Extract a Rust String from an FfiBuffer safely
///
/// # Arguments
/// * `buffer` - Reference to FfiBuffer
///
/// # Returns
/// String extracted from buffer, or empty string if buffer is invalid
///
/// # Safety
/// Dereferences the raw pointer in FfiBuffer. Caller must ensure:
/// - Pointer is valid
/// - Data is UTF-8 encoded (invalid UTF-8 is converted lossy)
/// - Buffer lifetime is valid
pub fn ffi_buffer_to_string(buffer: &FfiBuffer) -> String {
    if !buffer.is_valid() {
        return String::new();
    }

    if buffer.data.is_null() {
        return String::new();
    }

    unsafe {
        // SAFETY: FfiBuffer guarantees valid pointer and length
        let slice = slice::from_raw_parts(buffer.data, buffer.len);
        String::from_utf8_lossy(slice).into_owned()
    }
}

/// Allocate and write a u64 handle to a buffer
///
/// # Arguments
/// * `handle` - u64 handle to store
///
/// # Returns
/// FfiBuffer pointing to 8-byte buffer containing the handle
pub fn create_handle_buffer(handle: u64) -> FfiBuffer {
    let boxed_handle = Box::new(handle);
    let ptr = Box::into_raw(boxed_handle) as *mut u8;

    FfiBuffer {
        data: ptr,
        len: 8,
        cap: 8,
        _padding: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ffi_buffer_to_string_empty() {
        let buffer = FfiBuffer::new();
        let string = ffi_buffer_to_string(&buffer);
        assert_eq!(string, "");
    }

    #[test]
    fn test_ffi_buffer_to_string_valid() {
        let test_str = "hello, world!";
        let bytes = test_str.as_bytes();
        let boxed = bytes.to_vec().into_boxed_slice();
        let ptr = Box::into_raw(boxed) as *mut u8;

        let buffer = FfiBuffer {
            data: ptr,
            len: test_str.len(),
            cap: test_str.len(),
            _padding: 0,
        };

        let string = ffi_buffer_to_string(&buffer);
        assert_eq!(string, test_str);

        // Cleanup
        unsafe {
            let _ = Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, test_str.len()));
        }
    }

    #[test]
    fn test_handle_buffer_creation() {
        let handle = 12345u64;
        let buffer = create_handle_buffer(handle);

        assert_eq!(buffer.len, 8);
        assert_eq!(buffer.cap, 8);
        assert!(!buffer.data.is_null());

        // Verify contents
        unsafe {
            let stored_handle = *(buffer.data as *mut u64);
            assert_eq!(stored_handle, handle);
            let _ = Box::from_raw(buffer.data as *mut u64);
        }
    }
}
