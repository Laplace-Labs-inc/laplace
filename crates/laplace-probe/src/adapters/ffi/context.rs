//! # FFI Context Injection Module
//!
//! Implements `laplace_probe_inject_context` — the FFI entry point that allows
//! foreign AI agents (Python / TypeScript / Deno) to push a [`LaplaceContext`]
//! into a running [`MeshAgent`] before sending payloads.
//!
//! ## Usage pattern
//!
//! 1. Call `laplace_probe_init` to initialize the global runtime.
//! 2. Build a `MeshAgent` and register it in the global [`MeshAgentRegistry`]
//!    (the returned handle is the `agent_handle` used below).
//! 3. Call `laplace_probe_inject_context(agent_handle, ctx_ptr)` whenever the
//!    trace ID / tenant ID / clock changes.
//! 4. Call your normal send operations; the context is automatically stamped
//!    on every outbound frame until you call `inject_context` again.

use laplace_interfaces::{FfiBuffer, FfiResponse, LaplaceError};

use crate::domain::context::{FfiLaplaceContext, LaplaceContext};
use crate::infrastructure::runtime::{get_mesh_agent_registry, get_runtime};

/// Inject a [`LaplaceContext`] into the [`MeshAgent`] identified by `agent_handle`.
///
/// The context is stored in the agent and automatically stamped on every
/// subsequent outbound frame (with `virtual_clock_ns` overridden by the
/// agent's clock provider and `lamport_tick` auto-incremented).
///
/// # Arguments
///
/// - `agent_handle`: Handle returned by registering a `MeshAgent` in the
///   global [`MeshAgentRegistry`].
/// - `ctx_ptr`: Pointer to a caller-allocated `FfiLaplaceContext` (48 bytes,
///   C-ABI layout).  Must be non-null and validly initialized.
///
/// # Returns
///
/// `FfiResponse` with `error_code == 0` on success, or a non-zero error code:
/// - `LaplaceError::InvalidPointer`: `ctx_ptr` is null.
/// - `LaplaceError::InvalidContext`: `agent_handle` not found in registry.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn inject_context(
    agent_handle: u64,
    ctx_ptr: *const FfiLaplaceContext,
) -> Result<FfiResponse, LaplaceError> {
    if ctx_ptr.is_null() {
        return Ok(FfiResponse::error(LaplaceError::InvalidPointer as u32));
    }

    // Safety: caller guarantees valid, aligned pointer
    let ffi_ctx: FfiLaplaceContext = unsafe { *ctx_ptr };
    let ctx: LaplaceContext = ffi_ctx.into();

    let registry = get_mesh_agent_registry();
    let agent = registry
        .get(agent_handle)
        .ok_or(LaplaceError::InvalidContext)?;

    let runtime = get_runtime();
    runtime.block_on(agent.inject_context(ctx));

    Ok(FfiResponse::success(FfiBuffer::new()))
}
