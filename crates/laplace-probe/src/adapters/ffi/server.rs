//! # FFI Server Management Module
//!
//! Implements server lifecycle operations: creation, startup, and graceful shutdown.
//! Called via `laplace_probe_start` and `laplace_probe_stop` FFI functions.

use crate::adapters::quinn::server::QuicServer;
use crate::infrastructure::ffi::create_handle_buffer;
use crate::infrastructure::runtime::{get_registry, get_runtime};
use laplace_interfaces::{FfiBuffer, FfiQuicConfig, FfiResponse, LaplaceError};

/// Create and start a new QUIC server with the specified configuration.
///
/// This function creates a fresh QUIC server instance, registers it in the global
/// registry, and spawns an asynchronous initialization task on the tokio runtime.
/// The function returns immediately with a server handle, while actual server startup
/// occurs asynchronously.
///
/// ## Registration-First Design
///
/// The server is registered in the global registry immediately upon creation, enabling
/// the Deno SDK to reference the server before the asynchronous startup process completes.
/// This design allows the caller to begin using the server handle (e.g., sending packets)
/// while the actual network socket is being bound and configured. If operations are
/// attempted before startup completes, they may encounter a temporarily-unavailable
/// condition that typically resolves within milliseconds.
///
/// ## Asynchronous Initialization
///
/// The actual server startup (socket binding, TLS handshake acceptance, etc.) happens
/// on the tokio runtime asynchronously. The caller can check `laplace_probe_get_stats` to
/// determine if the server is fully running, or implement exponential backoff retry logic
/// for the first few operations.
///
/// ## Arguments
///
/// - `config`: A reference to a validated `FfiQuicConfig` structure.
///   The configuration is read but not retained; the function dereferences
///   and copies data synchronously.
///
/// ## Configuration Validation
///
/// The configuration must have been validated by the caller before this function is invoked.
/// This function assumes the configuration is valid and focuses on server creation and registration.
///
/// ## Returns
///
/// An `FfiResponse` with the following semantics:
///
/// - **Success** (`error_code == 0`): The server has been created and registered.
///   The `result_buffer` contains an 8-byte server handle (u64 in little-endian format)
///   that the caller must use in subsequent operations.
///
/// - **Failure** (`error_code != 0`): Server creation failed. Possible errors:
///   - `LaplaceError::InvalidRequest`: The configuration is invalid (port, address, or
///     certificate paths are malformed).
///   - `LaplaceError::Internal`: Runtime or registry access failed; indicates a severe
///     system error.
///
/// ## Safety Properties
///
/// - The configuration is read synchronously and never retained after the function returns.
/// - The returned server handle is valid for use in all subsequent server management functions.
/// - All operations are wrapped in `catch_unwind` by the caller to prevent panics.
///
/// ## Example: Deno Integration
///
/// ```typescript
/// const config = new Uint8Array(/* FfiQuicConfig structure */);
/// const startResponse = laplace.laplace_probe_start(config);
///
/// if (startResponse.error_code !== 0) {
///     console.error(`Failed to start server: ${startResponse.error_code}`);
///     return;
/// }
///
/// // Extract server handle from result_buffer
/// const handleBytes = new Uint8Array(8);
/// // Copy from startResponse.result_buffer...
/// const serverHandle = BigInt(handleBytes[0]) | (BigInt(handleBytes[1]) << 8n) | ...;
///
/// // Use server handle in subsequent calls
/// ```
///
/// ## Lifecycle Management
///
/// The returned server handle must eventually be released by calling `laplace_probe_stop`
/// to gracefully shut down the server and free resources. However, even if `laplace_probe_stop`
/// is not called, the server will be cleaned up when the Laplace process exits.
///
/// ## Performance Characteristics
///
/// This function completes in O(1) time after validation and async task spawning.
/// The actual server startup (binding socket, etc.) adds approximately 1-10 milliseconds
/// and occurs asynchronously on the runtime.
///
/// @public-todo: Replace with domain trait factory in Phase 4
/// Currently uses `QuicServer::new()` directly. Phase 4 will introduce a
/// `TransportFactory` trait allowing runtime selection between Quinn and custom engines.
pub fn start_server(config: &FfiQuicConfig) -> Result<FfiResponse, LaplaceError> {
    // Validate configuration
    if !config.is_valid() {
        return Ok(FfiResponse::error(LaplaceError::InvalidRequest as u32));
    }

    // @public-todo: Replace with domain trait factory in Phase 4
    // Create server instance with production-default DI backends
    let server = QuicServer::new_production(0, config.clone());

    // Register server in global registry
    let registry = get_registry();
    let handle = registry.register(server);

    // Spawn asynchronous startup task
    let runtime = get_runtime();
    runtime.spawn(async move {
        if let Some(server) = registry.get(handle) {
            match server.start().await {
                Ok(()) => {
                    eprintln!(
                        "[LAPLACE-PROBE-FFI] Server handle={} started successfully",
                        handle
                    );
                }
                Err(e) => {
                    eprintln!(
                        "[LAPLACE-PROBE-FFI] Server handle={} startup failed: {:?}",
                        handle, e
                    );
                }
            }
        }
    });

    // Return handle to caller (startup happens asynchronously)
    let handle_buffer = create_handle_buffer(handle);
    Ok(FfiResponse::success(handle_buffer))
}

/// Stop a running QUIC server and release its resources.
///
/// This function gracefully shuts down the specified server, closes all active client
/// connections, drains pending operations, and releases allocated resources. The server
/// is removed from the global registry immediately, preventing new operations on the handle.
/// The actual shutdown process occurs asynchronously on the tokio runtime.
///
/// ## Graceful Shutdown Process
///
/// The function calls the server's asynchronous `stop()` method, which:
/// 1. Stops accepting new client connections
/// 2. Sends connection closure frames to all active clients
/// 3. Waits for graceful close to complete (with timeout)
/// 4. Releases all network resources
///
/// This typically completes within 10-100 milliseconds per active connection.
///
/// ## Handle Invalidation
///
/// After this function returns successfully, the `server_handle` parameter is invalid
/// and must not be used in subsequent calls. Using an invalid handle will result in
/// a `LaplaceError::InvalidPointer` response.
///
/// ## Arguments
///
/// - `server_handle`: The server handle returned by `laplace_probe_start`. Must refer
///   to a currently registered server instance.
///
/// ## Returns
///
/// An `FfiResponse` with the following semantics:
///
/// - **Success** (`error_code == 0`): The server has been removed from the registry
///   and the asynchronous shutdown task has been spawned. The shutdown process will
///   complete on the tokio runtime.
///
/// - **Failure** (`error_code != 0`): The specified server could not be found.
///   Possible errors:
///   - `LaplaceError::InvalidPointer`: The `server_handle` does not refer to a
///     currently registered server (e.g., handle is stale, or server was already stopped).
///   - `LaplaceError::Internal`: Runtime or registry access failed.
///
/// ## Idempotency
///
/// Calling `stop_server` multiple times with the same handle is safe but the
/// second and subsequent calls will return an error because the server is no longer registered.
///
/// ## Safety Properties
///
/// - The handle is validated against the registry before shutdown is initiated.
/// - The function returns before the actual shutdown completes, avoiding long blocking calls.
///
/// ## Example: Deno Integration
///
/// ```typescript
/// const serverHandle = 12345n;  // From laplace_probe_start
///
/// const stopResponse = laplace.laplace_probe_stop(serverHandle);
/// if (stopResponse.error_code !== 0) {
///     console.error(`Failed to stop server: ${stopResponse.error_code}`);
///     return;
/// }
///
/// // Server is shut down; handle is now invalid
/// // Wait a moment for async shutdown to complete
/// await new Promise(resolve => setTimeout(resolve, 100));
/// ```
pub fn stop_server(server_handle: u64) -> Result<FfiResponse, LaplaceError> {
    let registry = get_registry();

    // Lookup and unregister server from registry
    let server = match registry.unregister(server_handle) {
        Some(s) => s,
        None => {
            return Ok(FfiResponse::error(LaplaceError::InvalidPointer as u32));
        }
    };

    // Spawn asynchronous shutdown task
    let runtime = get_runtime();
    runtime.spawn(async move {
        match server.stop().await {
            Ok(()) => {
                eprintln!(
                    "[LAPLACE-PROBE-FFI] Server handle={} stopped gracefully",
                    server_handle
                );
            }
            Err(e) => {
                eprintln!(
                    "[LAPLACE-PROBE-FFI] Server handle={} stop failed: {:?}",
                    server_handle, e
                );
            }
        }
    });

    Ok(FfiResponse::success(FfiBuffer::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stop_server_invalid_handle() {
        let result = stop_server(99999);
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.is_error());
        assert_eq!(response.error_code, LaplaceError::InvalidPointer as u32);
    }
}
