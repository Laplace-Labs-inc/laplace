// SPDX-License-Identifier: Apache-2.0
//! # FFI I/O Operations Module
//!
//! Implements packet transmission and statistics retrieval operations.
//! Called via `laplace_probe_send` and `laplace_probe_get_stats` FFI functions.

use crate::infrastructure::runtime::get_registry;
use crate::infrastructure::runtime::get_runtime;
use laplace_interfaces::{FfiBuffer, FfiResponse, LaplaceError, TransportPacket};

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// Retrieve current performance statistics from a running QUIC server.
///
/// This function collects a snapshot of the server's current performance metrics,
/// including connection counts, packet statistics, throughput measurements, and
/// latency information. The statistics are serialized to JSON format and returned
/// to the caller.
///
/// ## Statistics Collection
///
/// The function performs a synchronous snapshot of server metrics without blocking
/// the server's network I/O. Statistics include:
///
/// - Active connection count
/// - Total packets sent and received
/// - Bytes transferred (inbound and outbound)
/// - TLS handshake performance metrics
/// - Per-connection statistics if available
///
/// The returned snapshot is a point-in-time measurement and does not reflect
/// subsequent changes that occur while the caller is processing the response.
///
/// ## JSON Format
///
/// The statistics are returned as a UTF-8 encoded JSON object. The exact structure
/// is documented in the corresponding TypeScript type definitions in the Deno SDK.
/// The JSON is heap-allocated and ownership is transferred to the caller; the caller
/// is responsible for freeing the buffer via `js_free_buffer` when no longer needed.
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
/// - **Success** (`error_code == 0`): Statistics have been successfully collected
///   and serialized. The `result_buffer` contains a pointer to a heap-allocated
///   JSON string (UTF-8 encoded) with length and capacity fields set appropriately.
///
/// - **Failure** (`error_code != 0`): Statistics retrieval failed. Possible errors:
///   - `LaplaceError::InvalidPointer`: The `server_handle` does not refer to a
///     currently registered server.
///   - `LaplaceError::Internal`: Serialization failed (memory allocation or JSON
///     encoding error).
///
/// ## Memory Ownership
///
/// The returned buffer is owned by the caller. The Deno SDK must call the
/// corresponding `js_free_buffer` FFI function to deallocate the memory when done.
/// Failure to free the buffer will result in a memory leak.
///
/// ## Example: Deno Integration
///
/// ```typescript
/// const statsResponse = laplace.laplace_probe_get_stats(serverHandle);
///
/// if (statsResponse.error_code !== 0) {
///     console.error(`Failed to get stats: ${statsResponse.error_code}`);
///     return;
/// }
///
/// // Read JSON from result_buffer
/// const jsonBytes = new Uint8Array(
///     memory.buffer,
///     statsResponse.result_buffer.data,
///     statsResponse.result_buffer.len
/// );
/// const jsonString = new TextDecoder().decode(jsonBytes);
/// const stats = JSON.parse(jsonString);
///
/// // Free the buffer
/// laplace.js_free_buffer(statsResponse.result_buffer.data);
///
/// console.log(`Active connections: ${stats.active_connections}`);
/// ```
///
/// ## Performance Characteristics
///
/// Statistics collection is O(n) in the number of active connections. For servers
/// with thousands of connections, this operation may take 1-10 milliseconds.
/// The function does not block the server's event loop.
pub fn get_server_stats(server_handle: u64) -> Result<FfiResponse, LaplaceError> {
    let registry = get_registry();

    // Lookup server in registry
    let server = match registry.get(server_handle) {
        Some(s) => s,
        None => {
            return Ok(FfiResponse::error(LaplaceError::InvalidPointer as u32));
        }
    };

    // Collect statistics synchronously
    let stats = server.get_stats();

    // Serialize to JSON
    let json_bytes = match stats.to_json_bytes() {
        Ok(bytes) => bytes,
        Err(_) => {
            return Ok(FfiResponse::error(LaplaceError::Internal as u32));
        }
    };

    // Transfer ownership to caller via heap allocation
    let boxed_bytes = json_bytes.into_boxed_slice();
    let len = boxed_bytes.len();
    let ptr = Box::into_raw(boxed_bytes) as *mut u8;

    let stats_buffer = FfiBuffer {
        data: ptr,
        len,
        cap: len,
        _padding: 0,
    };

    Ok(FfiResponse::success(stats_buffer))
}

/// Asynchronously transmit a packet to a specific client connection.
///
/// This function enqueues a data packet for transmission to a client identified
/// by the connection ID. The actual transmission happens asynchronously on the
/// tokio runtime. The function returns immediately after validation and enqueueing,
/// following the fire-and-forget pattern.
///
/// ## Fire-and-Forget Pattern
///
/// This function returns success or failure based on validation and successful
/// task spawning only. It does not wait for the packet to be transmitted or
/// acknowledged by the remote client. Transmission errors (e.g., connection
/// closed, timeout) are logged but not returned to the caller.
///
/// This design keeps the FFI boundary fast and non-blocking. If the caller requires
/// delivery confirmation or retransmission, those mechanisms should be implemented
/// at the application protocol level above this transport layer.
///
/// ## Payload Handling
///
/// The payload data is copied from the FFI boundary into a heap-allocated `Vec<u8>`
/// to ensure ownership transfer for the asynchronous task. This is necessary for
/// safety across the C ABI boundary; the caller may deallocate their buffer
/// immediately after the function returns.
///
/// Performance note (roadmap): Zero-copy transmission via shared memory regions
/// and reference counting is planned to avoid this copy, requiring FFI protocol
/// and lifetime management changes.
///
/// ## Arguments
///
/// - `server_handle`: The server handle returned by `laplace_probe_start`. Must refer
///   to a currently registered server instance.
///
/// - `connection_id`: The QUIC connection identifier. This identifies the specific
///   client connection within the server. The connection ID is provided to the caller
///   via the connection callback mechanism.
///
/// - `payload_ptr`: A pointer to the packet data. Must be valid and readable if
///   `payload_len > 0`. May be null if `payload_len == 0`.
///
/// - `payload_len`: The length of the payload in bytes. May be zero for empty packets.
///
/// ## Returns
///
/// An `FfiResponse` with the following semantics:
///
/// - **Success** (`error_code == 0`): The packet has been successfully enqueued for
///   transmission. The caller may immediately deallocate the payload buffer. No
///   guarantee is provided that the packet will reach the destination.
///
/// - **Failure** (`error_code != 0`): The packet could not be enqueued. Possible errors:
///   - `LaplaceError::InvalidPointer`: The `server_handle` is invalid.
///   - `LaplaceError::Internal`: Runtime or registry access failed.
///
/// ## Pointer Validation
///
/// Pointer validation is performed by the caller before invoking this function.
/// Empty payloads (`payload_len == 0`) with null pointers are accepted and enqueued
/// as valid empty packets.
///
/// ## Safety Properties
///
/// - Payload data is copied into owned memory immediately; original buffer lifetime
///   is not constrained.
/// - All operations are wrapped in `catch_unwind` by the caller to prevent panics.
/// - Connection ID is passed through without validation (validation occurs at server layer).
///
/// ## Example: Deno Integration
///
/// ```typescript
/// const serverHandle = 12345n;
/// const connectionId = 1n;  // From connection callback
/// const payload = new Uint8Array([0x48, 0x65, 0x6c, 0x6c, 0x6f]);  // "Hello"
///
/// const sendResponse = laplace.laplace_probe_send(
///     serverHandle,
///     connectionId,
///     payload,
///     payload.length
/// );
///
/// if (sendResponse.error_code !== 0) {
///     console.error(`Failed to send packet: ${sendResponse.error_code}`);
///     return;
/// }
///
/// // Packet is enqueued for transmission (not yet sent)
/// // Application should implement retransmission if needed
/// ```
///
/// ## Performance Characteristics
///
/// This function completes in O(n) time where n is the payload length (due to copying).
/// For typical packet sizes (100 bytes to 1400 bytes), execution time is less than one microsecond.
/// The actual transmission (stream creation, write to QUIC, close) occurs on the
/// runtime asynchronously and does not block the FFI boundary.
///
/// Performance note (roadmap): Replace payload copy with reference-counted shared buffer to avoid O(n) overhead.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "40_Probe_FFI",
        link = "LEP-0016-laplace-probe-ffi_barrier_and_deterministic_chaos"
    )
)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn send_packet(
    server_handle: u64,
    connection_id: u64,
    payload_ptr: *const u8,
    payload_len: usize,
) -> Result<FfiResponse, LaplaceError> {
    let registry = get_registry();

    // Lookup server in registry
    let server = match registry.get(server_handle) {
        Some(s) => s,
        None => {
            return Ok(FfiResponse::error(LaplaceError::InvalidPointer as u32));
        }
    };

    // Copy payload from FFI boundary into owned memory
    // Performance note: replace with reference-counted shared buffer to avoid this copy (roadmap).
    let data = if payload_len > 0 {
        unsafe { std::slice::from_raw_parts(payload_ptr, payload_len).to_vec() }
    } else {
        Vec::new()
    };

    // Create transport packet (domain type)
    let packet = TransportPacket::new(data, connection_id);

    // Spawn asynchronous send task on runtime
    let runtime = get_runtime();
    runtime.spawn(async move {
        match server.send_packet(packet).await {
            Ok(()) => {
                // Packet transmitted successfully at transport layer
                // No guarantee of delivery at application layer
            }
            Err(e) => {
                // Transmission failed; log but do not return error to caller
                eprintln!(
                    "[LAPLACE-PROBE-FFI] Send to connection {} failed: {:?}",
                    connection_id, e
                );
            }
        }
    });

    // Return success immediately (actual send is asynchronous)
    Ok(FfiResponse::success(FfiBuffer::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::runtime::init_global_runtime;
    use std::sync::Once;

    static INIT: Once = Once::new();

    fn setup() {
        INIT.call_once(|| {
            let _ = init_global_runtime();
        });
    }

    #[test]
    fn test_get_server_stats_invalid_handle() {
        setup();
        let result = get_server_stats(99999);
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.is_error());
        assert_eq!(response.error_code, LaplaceError::InvalidPointer as u32);
    }

    #[test]
    fn test_send_packet_invalid_handle() {
        setup();
        let result = send_packet(99999, 1, std::ptr::null(), 0);
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.is_error());
        assert_eq!(response.error_code, LaplaceError::InvalidPointer as u32);
    }
}
