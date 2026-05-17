// SPDX-License-Identifier: Apache-2.0
//! # FFI Handshake Module
//!
//! Implements ABI version verification and global runtime initialization.
//! Called via `laplace_probe_init` as the mandatory first FFI function.

use crate::infrastructure::runtime::init_global_runtime;
use laplace_interfaces::{FfiBuffer, FfiResponse, LaplaceError, FFI_ABI_VERSION};

/// Initialize the Laplace QUIC system and verify ABI compatibility.
///
/// This is the mandatory first FFI call that must be made before any other Laplace
/// functions can be used. It performs two critical tasks: verifying that the Deno SDK
/// and the Laplace Kernel are using compatible ABI versions, and initializing the global
/// runtime environment required for all asynchronous operations.
///
/// ## Initialization Contract
///
/// Before calling any other Laplace FFI functions (`laplace_probe_start`, `laplace_probe_stop`,
/// etc.), the caller must first call `laplace_probe_init` with the ABI version and verify
/// success via the returned FfiResponse.
///
/// ## ABI Version Verification
///
/// The Deno SDK passes its supported ABI version as the `version` parameter. This value
/// must match the `laplace_core::FFI_ABI_VERSION` constant. If versions mismatch, the
/// function returns an error response immediately without initializing any resources.
/// This prevents incompatible callers from initializing the system.
///
/// The version format follows semantic versioning: `(major << 16) | minor`. For example,
/// version 1.1.0 is encoded as `0x00010001`.
///
/// ## Runtime Initialization
///
/// If ABI versions match, the function calls `init_global_runtime()` to initialize the
/// global tokio runtime and server registry. This is an idempotent operation: if the
/// runtime is already initialized (e.g., from a previous call), subsequent calls return
/// `Ok(())` immediately. This provides safety and flexibility for various initialization
/// patterns in the Deno SDK.
///
/// ## Arguments
///
/// - `version`: The ABI version advertised by the Deno SDK, encoded as
///   `(major << 16) | minor`. For example, `0x00010001` for version 1.1.0.
///
/// ## Returns
///
/// An `FfiResponse` with the following semantics:
///
/// - **Success** (`error_code == 0`): The ABI versions match and the global runtime
///   has been successfully initialized. All subsequent Laplace FFI calls will succeed
///   (at least for this initialization requirement).
///
/// - **Failure** (`error_code != 0`): One of the following errors occurred:
///   - `LaplaceError::AbiMismatch`: The version parameter does not match
///     `laplace_core::FFI_ABI_VERSION`. No resources are allocated in this case.
///   - `LaplaceError::Internal`: Runtime initialization failed (extremely rare; typically
///     indicates resource exhaustion). The system should be considered unstable.
///
/// The `result_buffer` field in the response is always an empty buffer (all zeros)
/// for this function.
///
/// ## Safety Properties
///
/// This function is safe to call from Deno TypeScript:
///
/// - The `version` parameter is an immutable scalar (u32), safe to receive via FFI.
/// - The return value (`FfiResponse`) is a 40-byte stack-allocated structure, safe
///   to return by value across the FFI boundary.
/// - All internal state is protected by `OnceLock` (no unsafe code or data races).
/// - Panics (if any occur) are caught and converted to error responses by the wrapper.
///
/// ## Example: Deno Integration
///
/// ```typescript
/// const laplace = Deno.dlopen("./liblaplace_knul.so", {
///     laplace_probe_init: { parameters: ["u32"], result: "i32" },
///     // ... other FFI functions
/// });
///
/// const abiVersion = 0x00010001;  // Version 1.1.0
/// const response = laplace.laplace_probe_init(abiVersion);
///
/// if (response.error_code !== 0) {
///     throw new Error(`Initialization failed: ${response.error_code}`);
/// }
///
/// // Now safe to call other functions
/// const startResponse = laplace.laplace_probe_start(configPtr);
/// ```
///
/// ## Relationship to Other Functions
///
/// This function must be called before any of the following:
/// - `laplace_probe_start`: Creates and starts a QUIC server
/// - `laplace_probe_stop`: Stops a running server
/// - `laplace_probe_get_stats`: Retrieves server statistics
/// - `laplace_probe_send`: Sends a packet to a client
///
/// If any other function is called before successful `laplace_probe_init`, it will return
/// an error indicating that the runtime is not initialized.
pub fn init_quic(version: u32) -> Result<FfiResponse, LaplaceError> {
    // Verify ABI version compatibility
    if version != FFI_ABI_VERSION {
        return Ok(FfiResponse::error(LaplaceError::AbiMismatch as u32));
    }

    // Initialize global runtime and registry (idempotent)
    init_global_runtime()?;

    // Success: runtime ready, all subsequent FFI calls will work
    Ok(FfiResponse::success(FfiBuffer::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_quic_version_mismatch() {
        let wrong_version = FFI_ABI_VERSION.wrapping_add(1);
        let response = init_quic(wrong_version).unwrap();
        assert!(response.is_error());
        assert_eq!(response.error_code, LaplaceError::AbiMismatch as u32);
    }

    #[test]
    fn test_init_quic_success() {
        let response = init_quic(FFI_ABI_VERSION).unwrap();
        assert!(response.is_success());
        assert_eq!(response.error_code, 0);
    }

    #[test]
    fn test_init_quic_idempotent() {
        let resp1 = init_quic(FFI_ABI_VERSION).unwrap();
        assert!(resp1.is_success());

        let resp2 = init_quic(FFI_ABI_VERSION).unwrap();
        assert!(resp2.is_success());
    }
}
