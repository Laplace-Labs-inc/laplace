//! Domain Utilities and Global State Registry
//!
//! This module contains the implementation of global singletons (Time, Entropy)
//! and utility functions that power the domain layer.

#[cfg(not(kani))]
use super::entropy::{self, Entropy};
#[cfg(not(kani))]
use super::time;
use super::tracing::TracerBackend;
use std::sync::OnceLock;

/// Global entropy source registry
///
/// Uses `OnceLock` to ensure thread-safe initialization while allowing
/// test-time injection of deterministic entropy. Once initialized, the
/// entropy source remains fixed for the lifetime of the process.
///
/// # Initialization
///
/// On first access, defaults to `SystemEntropy` if no entropy source
/// has been explicitly registered. For Axiom/test scenarios, call
/// `set_global_entropy()` before any code that depends on entropy
/// to inject a deterministic source.
#[cfg(not(kani))]
static GLOBAL_ENTROPY: OnceLock<Box<dyn Entropy>> = OnceLock::new();

/// Register a global entropy source for the entire platform.
///
/// # Arguments
///
/// - `entropy`: An entropy implementation to use globally.
///
/// # Returns
///
/// - `Ok(())` if the entropy source was successfully registered.
/// - `Err(&'static str)` if the global entropy has already been initialized.
///
/// # Usage
///
/// Call this function early in test setup or Axiom initialization:
///
/// ```ignore
/// #[cfg(feature = "twin")]
/// use laplace_core::domain::{set_global_entropy, DeterministicEntropy};
///
/// let entropy = DeterministicEntropy::new(0xFEEDDEAD);
/// set_global_entropy(Box::new(entropy)).expect("Failed to set entropy");
/// ```
#[cfg(not(kani))]
pub fn set_global_entropy(entropy: Box<dyn Entropy>) -> Result<(), &'static str> {
    GLOBAL_ENTROPY
        .set(entropy)
        .map_err(|_| "Global entropy source already initialized")
}

/// Get a reference to the current global entropy source.
///
/// # Behavior
///
/// - Returns the registered entropy source if one has been set via `set_global_entropy()`.
/// - Otherwise, initializes and returns a default `SystemEntropy`.
///
/// # Thread Safety
///
/// This function is thread-safe and may be called concurrently from any context.
#[cfg(not(kani))]
fn get_entropy() -> &'static dyn Entropy {
    GLOBAL_ENTROPY
        .get_or_init(|| Box::new(entropy::SystemEntropy::new()))
        .as_ref()
}

/// Global tracer registry (optional convenience layer).
///
/// If your platform needs global tracing, register a tracer here.
/// Otherwise, create tracers locally per simulation or test context.
/// This follows the Deterministic Context principle by making tracing
/// explicitly injectable rather than implicitly global.
static GLOBAL_TRACER: OnceLock<Box<dyn TracerBackend>> = OnceLock::new();

/// Register a global tracer for the platform.
///
/// # Returns
/// - `Ok(())` if successfully registered.
/// - `Err(&'static str)` if the global tracer is already initialized.
pub fn set_global_tracer(tracer: Box<dyn TracerBackend>) -> Result<(), &'static str> {
    GLOBAL_TRACER
        .set(tracer)
        .map_err(|_| "Global tracer already initialized")
}

/// Get a mutable reference to the global tracer (if registered).
///
/// This is intentionally unsafe to discourage global mutable state.
/// For production, create local tracer instances per simulation.
///
/// # Safety
///
/// This function uses interior mutability. Concurrent writes to the
/// global tracer should be synchronized by the caller.
pub fn get_global_tracer_backend() -> Option<&'static dyn TracerBackend> {
    GLOBAL_TRACER.get().map(|b| b.as_ref())
}

/// Macro for logging events to a tracer with minimal overhead.
///
/// # Example
///
/// ```ignore
/// trace_event!(tracer, Memory {
///     operation: MemoryOperation::Read {
///         addr: Address(0x1000),
///         value: 42,
///         cache_hit: true,
///     }
/// });
/// ```
///
/// Note: This is provided as a pattern. Implement based on your tracer's API.
#[macro_export]
macro_rules! trace_event {
    ($tracer:expr, $event:expr) => {
        let _ = $tracer.append_event($event);
    };
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Public Entropy Utilities
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Public Entropy Utilities
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Generate a random u64 value using the global entropy source.
///
/// In production, this uses the operating system's randomness. In Axiom/test
/// environments, it uses the injected deterministic entropy.
///
/// # Example
///
/// ```ignore
/// use laplace_core::domain::next_random_u64;
///
/// let random_id = next_random_u64();
/// ```
#[inline]
pub fn next_random_u64() -> u64 {
    #[cfg(not(kani))]
    {
        get_entropy().next_u64()
    }
    #[cfg(kani)]
    {
        // Kani 증명 시에는 결정론적인 고정값(0)을 반환하거나
        // kani::any()를 사용할 수 있지만, 우선 0으로 차단합니다.
        0u64
    }
}

/// Fill a buffer with random bytes using the global entropy source.
///
/// # Arguments
///
/// - `dest`: A mutable byte buffer to fill with random data.
///
/// # Example
///
/// ```ignore
/// use laplace_core::domain::fill_random_bytes;
///
/// let mut key_material = [0u8; 32];
/// fill_random_bytes(&mut key_material);
/// ```
#[inline]
pub fn fill_random_bytes(dest: &mut [u8]) {
    #[cfg(not(kani))]
    {
        get_entropy().fill_bytes(dest);
    }
    #[cfg(kani)]
    {
        // Kani 모드에서는 버퍼를 0으로 채웁니다.
        for byte in dest {
            *byte = 0;
        }
    }
}

/// Generate a random value uniformly distributed in `[0, max)`.
///
/// # Arguments
///
/// - `max`: The upper bound (exclusive) for the generated value.
///
/// # Returns
///
/// A value `v` where `0 <= v < max`.
///
/// # Panics
///
/// If `max == 0`.
///
/// # Example
///
/// ```ignore
/// use laplace_core::domain::next_random_range;
///
/// let tenant_id = next_random_range(1000); // 0..1000
/// ```
#[inline]
pub fn next_random_range(max: u64) -> u64 {
    assert!(max > 0, "next_random_range: max must be > 0");
    #[cfg(not(kani))]
    {
        get_entropy().next_range(max)
    }
    #[cfg(kani)]
    {
        // 항상 0을 반환하여 시스템 호출을 차단합니다.
        0u64
    }
}
/// Generate a random UUID as a hexadecimal string.
///
/// Produces a 128-bit random value formatted as a canonical UUID string
/// (36 characters including hyphens).
///
/// # Implementation
///
/// Uses the global entropy source to fill 16 random bytes, then formats
/// them as a UUID v4-style string (without setting version/variant bits,
/// since this is fully random).
///
/// # Example
///
/// ```ignore
/// use laplace_core::domain::generate_random_uuid;
///
/// let session_id = generate_random_uuid();
/// assert_eq!(session_id.len(), 36);
/// ```
pub fn generate_random_uuid() -> String {
    let mut bytes = [0u8; 16];
    fill_random_bytes(&mut bytes);
    format_uuid_string(&bytes)
}

/// Format a 16-byte array as a UUID string.
///
/// # Arguments
///
/// - `bytes`: A 16-byte buffer containing the UUID data.
///
/// # Returns
///
/// A string in the format `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`.
///
/// # Example
///
/// ```ignore
/// let bytes = [
///     0x12, 0x34, 0x56, 0x78,
///     0x9a, 0xbc, 0xde, 0xf0,
///     0x11, 0x22, 0x33, 0x44,
///     0x55, 0x66, 0x77, 0x88,
/// ];
/// let uuid = format_uuid_string(&bytes);
/// // Result: "12345678-9abc-def0-1122-334455667788"
/// ```
fn format_uuid_string(bytes: &[u8; 16]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Time Utilities
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(not(kani))]
static GLOBAL_CLOCK: OnceLock<Box<dyn time::Clock>> = OnceLock::new();

/// Initialize the global time source with a custom implementation.
///
/// Call this early in Axiom initialization or test setup to inject
/// a VirtualClock for deterministic timing.
#[cfg(not(kani))]
pub fn set_global_clock(clock: Box<dyn time::Clock>) -> Result<(), &'static str> {
    GLOBAL_CLOCK
        .set(clock)
        .map_err(|_| "Global clock already initialized")
}

/// Get a reference to the currently active time source.
///
/// This function is only compiled in non-Kani mode. In Kani verification,
/// it is completely excluded to prevent access to system time functions.
#[cfg(not(kani))]
fn get_clock() -> &'static dyn time::Clock {
    GLOBAL_CLOCK
        .get_or_init(|| Box::new(time::SystemClock::new()))
        .as_ref()
}

/// Get current time in milliseconds.
///
/// In production mode, returns actual elapsed time since program start.
/// In Kani verification mode, returns 0 (deterministic for formal verification).
#[inline]
pub fn now_ms() -> i64 {
    #[cfg(not(kani))]
    {
        get_clock().now_ms()
    }
    #[cfg(kani)]
    {
        // In Kani mode, return deterministic time value
        // This prevents access to system time APIs unsupported by symbolic execution
        0i64
    }
}

/// Get current time in microseconds.
///
/// In production mode, returns actual elapsed time since program start.
/// In Kani verification mode, returns 0 (deterministic for formal verification).
#[inline]
pub fn now_us() -> i64 {
    #[cfg(not(kani))]
    {
        get_clock().now_us() as i64
    }
    #[cfg(kani)]
    {
        // In Kani mode, return deterministic time value
        // This prevents access to system time APIs unsupported by symbolic execution
        0i64
    }
}

/// Get current time in nanoseconds.
///
/// In production mode, returns actual elapsed time since program start.
/// In Kani verification mode, returns 0 (deterministic for formal verification).
///
/// This function is critical for scheduler verification - it must return
/// a deterministic value during formal verification to avoid triggering
/// unsupported system calls (clock_gettime) in the symbolic execution engine.
#[inline]
pub fn now_ns() -> i64 {
    #[cfg(not(kani))]
    {
        get_clock().now_ns() as i64
    }
    #[cfg(kani)]
    {
        // In Kani mode, return deterministic time value (0).
        // This is safe because Kani verification is purely functional:
        // - TaskId allocation doesn't depend on elapsed time
        // - All scheduling logic uses virtual time (ClockBackend)
        // - No timing-based branching occurs in scheduler engine
        //
        // Returning 0 here prevents the call chain that would reach
        // std::time::Instant::now() -> clock_gettime (unsupported in Kani)
        0i64
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Domain Constants
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Default pool size for resource allocation
pub const DEFAULT_POOL_SIZE: usize = 100;

/// Default idle timeout for pooled resources in seconds
pub const DEFAULT_IDLE_TIMEOUT_SECS: u64 = 300;

/// Turbo acceleration latency target in nanoseconds
///
/// Zero-copy shared memory should achieve context synchronization
/// latency below this threshold.
pub const TURBO_LATENCY_TARGET_NS: u64 = 500;

/// Standard FFI latency baseline in nanoseconds
///
/// Protobuf FFI path typically achieves approximately 41.5µs
/// context synchronization overhead.
pub const STANDARD_LATENCY_BASELINE_NS: u64 = 41_500;
