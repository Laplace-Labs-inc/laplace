//! # FFI Adapter Layer - Laplace QUIC Interface
//!
//! This module provides the complete set of FFI entry points for communication between
//! the Deno TypeScript SDK and the Laplace Kernel. All functions follow the extern "C"
//! calling convention and are protected by panic-safe wrappers to prevent Rust panics
//! from propagating across the FFI boundary.
//!
//! ## Architecture
//!
//! The FFI adapter is organized into four logical modules:
//!
//! - **handshake**: ABI version verification and runtime initialization (`laplace_probe_init`)
//! - **server**: Server lifecycle management (`laplace_probe_start`, `laplace_probe_stop`)
//! - **io**: Packet transmission and statistics retrieval (`laplace_probe_send`, `laplace_probe_get_stats`)
//! - **mod.rs** (this file): Thin wrappers around implementation functions, providing panic safety
//!
//! Each implementation module exports an interior function returning `Result<FfiResponse, LaplaceError>`.
//! The thin wrappers in this file call these functions within `catch_unwind` blocks to convert
//! Rust panics into error responses, then convert error codes using `as u32` casting.
//!
//! ## Safety Contract
//!
//! - **Initialization First**: `laplace_probe_init` validates ABI compatibility and initializes
//!   global runtime. All other functions require successful prior initialization.
//!
//! - **Panic Safety**: Every FFI entry point is wrapped in `catch_unwind(AssertUnwindSafe(...))`.
//!   Rust panics are converted to error responses; the system remains stable.
//!
//! - **Fire-and-Forget Pattern**: Server control and I/O functions return immediately after
//!   validation and async task spawning. Actual work happens on the tokio runtime asynchronously.
//!
//! - **Pointer Validation**: All FFI pointers are validated before dereferencing.
//!   Configuration validity is checked before creating server instances.

pub mod context;
pub mod handshake;
pub mod io;
pub mod server;

use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::domain::context::FfiLaplaceContext;
use laplace_interfaces::{FfiQuicConfig, FfiResponse, LaplaceError};

// ============================================================================
// FFI Handshake & Initialization - Thin Wrapper
// ============================================================================

/// Initialize the Laplace QUIC system and verify ABI compatibility.
///
/// This is the mandatory first FFI call that must be made before any other Laplace
/// functions can be used. It verifies ABI version compatibility and initializes
/// the global runtime environment.
///
/// **Returns**: `FfiResponse` with `error_code == 0` on success, or an error code
/// on version mismatch or runtime initialization failure.
#[no_mangle]
pub extern "C" fn laplace_probe_init(version: u32) -> FfiResponse {
    let result = catch_unwind(AssertUnwindSafe(|| handshake::init_quic(version)));

    match result {
        Ok(inner_result) => match inner_result {
            Ok(response) => response,
            Err(e) => FfiResponse::error(e as u32),
        },
        Err(_) => {
            eprintln!("[LAPLACE-PROBE-FFI] laplace_probe_init panicked unexpectedly");
            FfiResponse::error(LaplaceError::Internal as u32)
        }
    }
}

// ============================================================================
// FFI Server Management - Thin Wrappers
// ============================================================================

/// Create and start a new QUIC server with the specified configuration.
///
/// Creates a fresh QUIC server instance, registers it in the global registry,
/// and spawns an asynchronous initialization task. Returns immediately with a
/// server handle while actual startup occurs asynchronously.
///
/// **Arguments**:
/// - `config_ptr`: Pointer to `FfiQuicConfig` structure
///
/// **Returns**: `FfiResponse` with server handle on success, error code on failure.
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn laplace_probe_start(config_ptr: *const FfiQuicConfig) -> FfiResponse {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if config_ptr.is_null() {
            return Err(LaplaceError::InvalidPointer);
        }
        let config = unsafe { &*config_ptr };
        server::start_server(config)
    }));

    match result {
        Ok(inner_result) => match inner_result {
            Ok(response) => response,
            Err(e) => FfiResponse::error(e as u32),
        },
        Err(_) => {
            eprintln!("[LAPLACE-PROBE-FFI] laplace_probe_start panicked unexpectedly");
            FfiResponse::error(LaplaceError::Internal as u32)
        }
    }
}

/// Stop a running QUIC server and release its resources.
///
/// Gracefully shuts down the specified server, closes all active client connections,
/// and releases allocated resources. The server is removed from the registry immediately.
/// Actual shutdown process occurs asynchronously.
///
/// **Arguments**:
/// - `server_handle`: Server handle returned by `laplace_probe_start`
///
/// **Returns**: `FfiResponse` with `error_code == 0` on success, error code on failure.
#[no_mangle]
pub extern "C" fn laplace_probe_stop(server_handle: u64) -> FfiResponse {
    let result = catch_unwind(AssertUnwindSafe(|| server::stop_server(server_handle)));

    match result {
        Ok(inner_result) => match inner_result {
            Ok(response) => response,
            Err(e) => FfiResponse::error(e as u32),
        },
        Err(_) => {
            eprintln!("[LAPLACE-PROBE-FFI] laplace_probe_stop panicked unexpectedly");
            FfiResponse::error(LaplaceError::Internal as u32)
        }
    }
}

// ============================================================================
// FFI I/O Operations - Thin Wrappers
// ============================================================================

/// Retrieve current performance statistics from a running QUIC server.
///
/// Collects a snapshot of server metrics (connection counts, packet statistics,
/// throughput measurements) and serializes them to JSON format.
///
/// **Arguments**:
/// - `server_handle`: Server handle returned by `laplace_probe_start`
///
/// **Returns**: `FfiResponse` with JSON statistics in `result_buffer` on success,
/// error code on failure.
#[no_mangle]
pub extern "C" fn laplace_probe_get_stats(server_handle: u64) -> FfiResponse {
    let result = catch_unwind(AssertUnwindSafe(|| io::get_server_stats(server_handle)));

    match result {
        Ok(inner_result) => match inner_result {
            Ok(response) => response,
            Err(e) => FfiResponse::error(e as u32),
        },
        Err(_) => {
            eprintln!("[LAPLACE-PROBE-FFI] laplace_probe_get_stats panicked unexpectedly");
            FfiResponse::error(LaplaceError::Internal as u32)
        }
    }
}

/// Asynchronously transmit a packet to a specific client connection.
///
/// Enqueues a data packet for transmission to a client identified by the connection ID.
/// Actual transmission occurs asynchronously on the tokio runtime. Function returns
/// immediately following the fire-and-forget pattern.
///
/// **Arguments**:
/// - `server_handle`: Server handle returned by `laplace_probe_start`
/// - `connection_id`: QUIC connection identifier
/// - `payload_ptr`: Pointer to packet data
/// - `payload_len`: Length of packet data in bytes
///
/// **Returns**: `FfiResponse` with `error_code == 0` on success, error code on failure.
#[no_mangle]
pub extern "C" fn laplace_probe_send(
    server_handle: u64,
    connection_id: u64,
    payload_ptr: *const u8,
    payload_len: usize,
) -> FfiResponse {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if payload_ptr.is_null() && payload_len > 0 {
            return Err(LaplaceError::InvalidPointer);
        }
        io::send_packet(server_handle, connection_id, payload_ptr, payload_len)
    }));

    match result {
        Ok(inner_result) => match inner_result {
            Ok(response) => response,
            Err(e) => FfiResponse::error(e as u32),
        },
        Err(_) => {
            eprintln!("[LAPLACE-PROBE-FFI] laplace_probe_send panicked unexpectedly");
            FfiResponse::error(LaplaceError::Internal as u32)
        }
    }
}

// ============================================================================
// FFI Context Injection
// ============================================================================

/// Inject a `LaplaceContext` into the `MeshAgent` identified by `agent_handle`.
///
/// Allows foreign AI agents (Python / TypeScript / Deno) to set the active
/// tracing context before sending payloads.  The context is automatically
/// stamped on every subsequent outbound frame.
///
/// **Arguments**:
/// - `agent_handle`: Handle obtained when registering a `MeshAgent`
/// - `ctx_ptr`: Non-null pointer to a caller-allocated `FfiLaplaceContext` (48 bytes)
///
/// **Returns**: `FfiResponse` with `error_code == 0` on success.
#[no_mangle]
pub extern "C" fn laplace_probe_inject_context(
    agent_handle: u64,
    ctx_ptr: *const FfiLaplaceContext,
) -> FfiResponse {
    let result = catch_unwind(AssertUnwindSafe(|| {
        context::inject_context(agent_handle, ctx_ptr)
    }));

    match result {
        Ok(inner) => match inner {
            Ok(response) => response,
            Err(e) => FfiResponse::error(e as u32),
        },
        Err(_) => {
            eprintln!("[LAPLACE-PROBE-FFI] laplace_probe_inject_context panicked unexpectedly");
            FfiResponse::error(LaplaceError::Internal as u32)
        }
    }
}

// ============================================================================
// Kani Proofs
// ============================================================================

#[cfg(kani)]
mod proofs {
    use laplace_interfaces::LaplaceError;

    /// H-KNUL4: LaplaceError `as u32` conversion is always within [0, 0xFFFF].
    ///
    /// Proves that no defined LaplaceError variant can produce a garbage value or
    /// integer overflow when cast to u32 for FFI transmission. Uses an index-based
    /// symbolic enumeration because LaplaceError is a #[repr(u32)] C-compatible enum
    /// without an Arbitrary impl. The maximum defined discriminant is
    /// SchedulerError = 6002 = 0x1772, well within the 16-bit safe range.
    #[kani::proof]
    fn ffi_error_code_bounded() {
        let idx: u8 = kani::any();
        kani::assume(idx < 26);

        let error = match idx {
            0 => LaplaceError::Success,
            1 => LaplaceError::Internal,
            2 => LaplaceError::InvalidContext,
            3 => LaplaceError::AbiMismatch,
            4 => LaplaceError::MemoryAlignment,
            5 => LaplaceError::InvalidPointer,
            6 => LaplaceError::Timeout,
            7 => LaplaceError::KernelTimeout,
            8 => LaplaceError::SdkTimeout,
            9 => LaplaceError::LockTimeout,
            10 => LaplaceError::QuotaExceeded,
            11 => LaplaceError::OutOfMemory,
            12 => LaplaceError::CpuQuotaExceeded,
            13 => LaplaceError::ConcurrencyLimitExceeded,
            14 => LaplaceError::PoolExhausted,
            15 => LaplaceError::HandshakeFailed,
            16 => LaplaceError::InvalidRequest,
            17 => LaplaceError::VersionMismatch,
            18 => LaplaceError::TenantNotFound,
            19 => LaplaceError::Unauthorized,
            20 => LaplaceError::NetworkError,
            21 => LaplaceError::ConnectionFailed,
            22 => LaplaceError::SerializationError,
            23 => LaplaceError::VerificationError,
            24 => LaplaceError::IsolatePoolError,
            _ => LaplaceError::SchedulerError,
        };

        let code = error as u32;
        assert!(
            code <= 0xFFFF,
            "FFI error code must never exceed 0xFFFF to prevent overflow at FFI boundary"
        );
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use laplace_interfaces::FFI_ABI_VERSION;

    #[test]
    fn test_laplace_probe_init_version_mismatch() {
        let wrong_version = FFI_ABI_VERSION.wrapping_add(1);
        let response = laplace_probe_init(wrong_version);
        assert!(response.is_error());
        assert_eq!(response.error_code, LaplaceError::AbiMismatch as u32);
    }

    #[test]
    fn test_laplace_probe_init_success() {
        let response = laplace_probe_init(FFI_ABI_VERSION);
        assert!(response.is_success());
    }

    #[test]
    fn test_laplace_probe_start_null_config() {
        let _ = laplace_probe_init(FFI_ABI_VERSION);
        let response = laplace_probe_start(std::ptr::null());
        assert!(response.is_error());
        assert_eq!(response.error_code, LaplaceError::InvalidPointer as u32);
    }

    #[test]
    fn test_laplace_probe_stop_invalid_handle() {
        let _ = laplace_probe_init(FFI_ABI_VERSION);
        let response = laplace_probe_stop(99999);
        assert!(response.is_error());
    }

    #[test]
    fn test_laplace_probe_get_stats_invalid_handle() {
        let _ = laplace_probe_init(FFI_ABI_VERSION);
        let response = laplace_probe_get_stats(99999);
        assert!(response.is_error());
    }

    #[test]
    fn test_laplace_probe_send_invalid_handle() {
        let _ = laplace_probe_init(FFI_ABI_VERSION);
        let response = laplace_probe_send(99999, 1, std::ptr::null(), 0);
        assert!(response.is_error());
    }

    #[test]
    fn test_laplace_probe_send_null_with_length() {
        let _ = laplace_probe_init(FFI_ABI_VERSION);
        let response = laplace_probe_send(1, 1, std::ptr::null(), 10);
        assert!(response.is_error());
    }
}
