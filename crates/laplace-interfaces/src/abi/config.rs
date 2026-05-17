//! # QUIC Server Configuration
//!
//! FFI-safe configuration structures for QUIC server initialization.
//! This module defines the contract for transport configuration across
//! the Rust-Deno boundary.

use super::primitives::FfiBuffer;
use super::FfiValidatable;
use serde::{Deserialize, Serialize};

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

/// QUIC Server Configuration
///
/// Passed to QUIC server startup via FFI boundary.
/// Contains both fixed-size fields and variable-length buffers via FfiBuffer.
///
/// **Memory Layout (144 bytes, 8-byte aligned)**:
/// - Offset 0: `port` (u16, 2 bytes)
/// - Offset 2: `_padding1` (u16, 2 bytes)
/// - Offset 4: `max_streams` (u32, 4 bytes)
/// - Offset 8: `idle_timeout_ms` (u64, 8 bytes)
/// - Offset 16: `host_addr` (FfiBuffer, 32 bytes)
/// - Offset 48: `cert_path` (FfiBuffer, 32 bytes)
/// - Offset 80: `key_path` (FfiBuffer, 32 bytes)
/// - Offset 112: `ca_cert_path` (FfiBuffer, 32 bytes)
///
/// Total: 144 bytes (8-byte aligned)
// [ABI_GUARD]: FFI Boundary
#[repr(C, align(8))]
#[derive(Debug, Clone)]
pub struct FfiQuicConfig {
    /// UDP port to listen on
    pub port: u16,

    /// Padding for alignment
    pub _padding1: u16,

    /// Maximum concurrent streams per connection
    pub max_streams: u32,

    /// Connection idle timeout in milliseconds
    pub idle_timeout_ms: u64,

    /// Host address to bind to (e.g., "127.0.0.1" or "0.0.0.0")
    /// FfiBuffer: Null-terminated string
    pub host_addr: FfiBuffer,

    /// Path to TLS certificate file (PEM format)
    /// FfiBuffer: Null-terminated path string
    pub cert_path: FfiBuffer,

    /// Path to TLS private key file (PEM format)
    /// FfiBuffer: Null-terminated path string
    pub key_path: FfiBuffer,

    /// Path to CA certificate bundle (PEM format) for client verification
    /// FfiBuffer: Null-terminated path string
    pub ca_cert_path: FfiBuffer,
}

impl FfiQuicConfig {
    /// Create a new QUIC configuration with defaults
    pub fn new() -> Self {
        Self {
            port: 0,
            _padding1: 0,
            max_streams: 1000,
            idle_timeout_ms: 120000,
            host_addr: FfiBuffer::new(),
            cert_path: FfiBuffer::new(),
            key_path: FfiBuffer::new(),
            ca_cert_path: FfiBuffer::new(),
        }
    }

    /// Verify configuration validity
    pub fn is_valid(&self) -> bool {
        // Basic port/limit validation
        if self.port == 0 || self.max_streams == 0 || self.idle_timeout_ms == 0 {
            return false;
        }

        // Verify buffer structural stability and data presence simultaneously
        self.host_addr.is_valid()
            && self.host_addr.len > 0
            && self.cert_path.is_valid()
            && self.cert_path.len > 0
            && self.key_path.is_valid()
            && self.key_path.len > 0
    }
}

impl Default for FfiQuicConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl FfiValidatable for FfiQuicConfig {
    fn is_valid(&self) -> bool {
        FfiQuicConfig::is_valid(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem;

    #[test]
    fn ffi_quic_config_size_and_align() {
        assert_eq!(
            mem::align_of::<FfiQuicConfig>(),
            8,
            "FfiQuicConfig must be 8-byte aligned"
        );
        // Size should be multiple of 8
        assert_eq!(
            mem::size_of::<FfiQuicConfig>() % 8,
            0,
            "FfiQuicConfig size must be multiple of 8"
        );
        assert_eq!(
            mem::size_of::<FfiQuicConfig>(),
            144,
            "FfiQuicConfig must be exactly 144 bytes"
        );
    }

    #[test]
    fn ffi_quic_config_creation() {
        let config = FfiQuicConfig::new();

        assert_eq!(config.port, 0);
        assert_eq!(config.max_streams, 1000);
        assert_eq!(config.idle_timeout_ms, 120000);
    }

    #[test]
    fn ffi_quic_config_validation_port_zero() {
        let config = FfiQuicConfig::new();
        assert!(!config.is_valid(), "Port 0 should be invalid");
    }

    #[test]
    fn ffi_quic_config_validation_zero_streams() {
        let mut config = FfiQuicConfig::new();
        config.port = 8080;
        config.max_streams = 0;
        assert!(!config.is_valid(), "Zero streams should be invalid");
    }

    #[test]
    fn ffi_quic_config_validation_zero_timeout() {
        let mut config = FfiQuicConfig::new();
        config.port = 8080;
        config.idle_timeout_ms = 0;
        assert!(!config.is_valid(), "Zero timeout should be invalid");
    }

    #[test]
    fn ffi_quic_config_validation_missing_buffers() {
        let mut config = FfiQuicConfig::new();
        config.port = 8080;
        // All buffers are empty/invalid, so config should be invalid
        assert!(!config.is_valid(), "Empty buffers should be invalid");
    }

    #[test]
    fn ffi_quic_config_valid_minimal() {
        let mut config = FfiQuicConfig::new();
        config.port = 8080;

        // Create valid but minimal FfiBuffers by setting data to non-null
        let dummy_data = [0u8; 10];
        config.host_addr = FfiBuffer {
            data: dummy_data.as_ptr() as *mut u8,
            len: 9,
            cap: 10,
            _padding: 0,
        };
        config.cert_path = FfiBuffer {
            data: dummy_data.as_ptr() as *mut u8,
            len: 1,
            cap: 10,
            _padding: 0,
        };
        config.key_path = FfiBuffer {
            data: dummy_data.as_ptr() as *mut u8,
            len: 1,
            cap: 10,
            _padding: 0,
        };

        assert!(
            config.is_valid(),
            "Config with valid port and buffers should be valid"
        );
    }

    #[test]
    fn ffi_quic_config_default() {
        let config = FfiQuicConfig::default();
        assert_eq!(config.port, 0);
        assert_eq!(config.max_streams, 1000);
    }

    #[test]
    fn ffi_quic_config_validatable_trait() {
        let config = FfiQuicConfig::new();
        assert!(!config.is_valid());
        // Trait method should agree with inherent method
        assert_eq!(
            FfiQuicConfig::is_valid(&config),
            <FfiQuicConfig as FfiValidatable>::is_valid(&config)
        );
    }
}

// ============================================================================
// Laplace Platform Configuration
// ============================================================================

/// Network layer configuration for the KNUL QUIC adapter.
///
/// Controls connection limits, timeouts, and bind settings for the
/// `laplace-knul` network adapter. Pure data — no I/O logic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Maximum number of concurrent QUIC connections.
    pub max_connections: u32,

    /// Connection idle timeout in milliseconds.
    pub idle_timeout_ms: u64,

    /// QUIC server bind address (e.g., `"127.0.0.1"` or `"0.0.0.0"`).
    pub bind_address: String,

    /// QUIC server UDP port.
    pub port: u16,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            max_connections: 1_000,
            idle_timeout_ms: 30_000,
            bind_address: "127.0.0.1".to_string(),
            port: 4433,
        }
    }
}

/// Verification engine configuration for the Axiom simulation layer.
///
/// Controls VU count, duration limits, and deterministic RNG settings
/// used by `laplace-kraken`. Pure data — no runtime logic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationConfig {
    /// Maximum number of virtual users (VUs) in a single simulation run.
    pub max_virtual_users: u32,

    /// Maximum simulation wall-clock duration in seconds.
    pub max_simulation_duration_secs: u64,

    /// Enable deterministic RNG for fully reproducible simulation runs.
    pub deterministic: bool,

    /// Global RNG seed. Only applied when `deterministic` is `true`.
    pub global_seed: u64,
}

impl Default for VerificationConfig {
    fn default() -> Self {
        Self {
            max_virtual_users: 100,
            max_simulation_duration_secs: 300,
            deterministic: true,
            global_seed: 0xDEAD_BEEF_CAFE_1337,
        }
    }
}

/// Tenant resource quota limits enforced by the sovereign kernel.
///
/// Defines per-tenant memory, thread, and event buffer ceilings.
/// Maps to `ResourceConfig` in the tenant domain for quota enforcement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimitConfig {
    /// Maximum heap memory per virtual user in bytes (default: 64 MiB).
    pub max_memory_per_vu_bytes: u64,

    /// Maximum number of concurrent scheduler threads.
    pub max_scheduler_threads: u32,

    /// Maximum number of tracing events retained in the causality buffer.
    pub max_trace_events: u32,
}

impl Default for ResourceLimitConfig {
    fn default() -> Self {
        Self {
            max_memory_per_vu_bytes: 64 * 1024 * 1024,
            max_scheduler_threads: 8,
            max_trace_events: 100_000,
        }
    }
}

/// Top-level Laplace platform configuration.
///
/// The single authoritative configuration contract for the entire Laplace
/// stack. Each sub-struct corresponds to a domain layer:
///
/// | Field          | Crate             | Purpose                           |
/// |----------------|-------------------|-----------------------------------|
/// | `network`      | `laplace-knul`     | QUIC adapter settings             |
/// | `verification` | `laplace-kraken`     | Simulation engine settings        |
/// | `resources`    | `laplace-core`     | Tenant resource quota limits      |
///
/// **No I/O here.** Deserialisation from `.toml`/`.json` is the responsibility
/// of the application layer (e.g., `laplace-core` or the CLI entry point).
///
/// The TUI display permissions are governed by
/// [`crate::domain::tui::TuiCapabilities`], which the kernel derives at
/// runtime from the active tenant tier and authentication state. The two
/// types are intentionally decoupled: `LaplaceConfig` drives *what the system
/// does*; `TuiCapabilities` drives *what the operator sees*.
///
/// # Example
///
/// ```ignore
/// let cfg = LaplaceConfig::default();
/// assert_eq!(cfg.network.port, 4433);
/// assert_eq!(cfg.verification.max_virtual_users, 100);
/// ```
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Config",
        link = "LEP-0001-laplace-interfaces-global_ffi_config"
    )
)]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LaplaceConfig {
    /// KNUL network adapter settings.
    pub network: NetworkConfig,

    /// Axiom verification engine settings.
    pub verification: VerificationConfig,

    /// Tenant resource quota limits.
    pub resources: ResourceLimitConfig,
}

// ============================================================================
// C-ABI Global Configuration  (LaplaceGlobalConfig)
// ============================================================================

/// Axiom deterministic verification engine parameters.
///
/// All tuning knobs for the DPOR / Ki-DPOR Oracle.
///
/// **Memory layout (24 bytes, 8-byte aligned)**:
/// ```text
/// Offset  Size  Field
///  0       4    max_threads              (u32)
///  4       4    max_depth                (u32)
///  8       4    max_starvation_limit     (u32)
/// 12       4    max_danger               (u32)
/// 16       8    default_seed             (u64)
/// ```
#[repr(C, align(8))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AxiomConfig {
    /// Maximum concurrent threads tracked by DPOR (≤ 64, TinyBitSet limit).
    pub max_threads: u32,
    /// Maximum exploration depth / step budget for Classic and Ki-DPOR.
    pub max_depth: u32,
    /// Maximum steps a thread may wait before Ki-DPOR flags a liveness violation.
    pub max_starvation_limit: u32,
    /// Upper bound on the heuristic danger score used by A*-Ki prioritisation.
    pub max_danger: u32,
    /// Master RNG seed for the Axiom Oracle; embedded in `.ard` headers for
    /// deterministic replay.
    pub default_seed: u64,
}

const _: () = assert!(
    core::mem::size_of::<AxiomConfig>() == 24,
    "AxiomConfig must be exactly 24 bytes"
);

impl Default for AxiomConfig {
    fn default() -> Self {
        Self {
            max_threads: 8,
            max_depth: 20,
            max_starvation_limit: 10,
            max_danger: 2_000,
            default_seed: 0xA110_0ACE_5EED_0001,
        }
    }
}

/// Kraken load-simulation engine parameters.
///
/// **Memory layout (40 bytes, 8-byte aligned)**:
/// ```text
/// Offset  Size  Field
///  0       4    max_response_bytes           (u32)
///  4       4    yield_interval_iterations    (u32)
///  8       8    max_safety_iterations        (u64)
/// 16       8    rng_scale                    (u64)
/// 24       4    network_base_latency_ms      (u32)
/// 28       4    network_jitter_ms            (u32)
/// 32       4    network_error_probability_ppm(u32)
/// 36       4    _padding                     ([u8;4])
/// ```
#[repr(C, align(8))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KrakenConfig {
    /// Maximum bytes read from a single agent response frame (default: 4 KiB).
    pub max_response_bytes: u32,
    /// Cooperative-scheduling yield interval: yield every N main-loop iterations.
    pub yield_interval_iterations: u32,
    /// Absolute safety cap on VU main-loop iterations to prevent infinite loops.
    pub max_safety_iterations: u64,
    /// Divisor for RNG → probability conversion (1 000 000 = 6 decimal places).
    pub rng_scale: u64,
    /// Baseline network latency injected per virtual request (milliseconds).
    pub network_base_latency_ms: u32,
    /// Peak-to-peak jitter added on top of the baseline latency (milliseconds).
    pub network_jitter_ms: u32,
    /// Simulated packet-error rate in parts-per-million (50 000 ppm = 5 %).
    pub network_error_probability_ppm: u32,
    /// Explicit ABI padding — always zero, not serialised.
    #[serde(skip)]
    pub _padding: [u8; 4],
}

const _: () = assert!(
    core::mem::size_of::<KrakenConfig>() == 40,
    "KrakenConfig must be exactly 40 bytes"
);

impl Default for KrakenConfig {
    fn default() -> Self {
        Self {
            max_response_bytes: 4_096,
            yield_interval_iterations: 256,
            max_safety_iterations: 5_000_000,
            rng_scale: 1_000_000,
            network_base_latency_ms: 10,
            network_jitter_ms: 2,
            network_error_probability_ppm: 0,
            _padding: [0u8; 4],
        }
    }
}

/// Probe QUIC mesh-sidecar parameters.
///
/// **Memory layout (32 bytes, 8-byte aligned)**:
/// ```text
/// Offset  Size  Field
///  0       4    max_frame_len              (u32)
///  4       4    batch_size                 (u32)
///  8       8    flush_interval_ms          (u64)
/// 16       4    lz4_compression_threshold  (u32)
/// 20       4    promotion_threshold        (u32)
/// 24       2    bind_port                  (u16)
/// 26       6    _padding                   ([u8;6])
/// ```
#[repr(C, align(8))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeConfig {
    /// Maximum total inbound frame size accepted over QUIC (default: 4 MiB).
    pub max_frame_len: u32,
    /// Number of messages accumulated before forcing a batch flush.
    pub batch_size: u32,
    /// Maximum idle time before forcing a batch flush (milliseconds).
    pub flush_interval_ms: u64,
    /// Payload size above which Layer-3 LZ4 compression is applied (bytes).
    pub lz4_compression_threshold: u32,
    /// Minimum observation count before a token is promoted to the dynamic dict.
    pub promotion_threshold: u32,
    /// UDP port the Probe sidecar binds on.
    pub bind_port: u16,
    /// Explicit ABI padding — always zero, not serialised.
    #[serde(skip)]
    pub _padding: [u8; 6],
}

const _: () = assert!(
    core::mem::size_of::<ProbeConfig>() == 32,
    "ProbeConfig must be exactly 32 bytes"
);

impl Default for ProbeConfig {
    fn default() -> Self {
        Self {
            max_frame_len: 4 * 1024 * 1024,
            batch_size: 64,
            flush_interval_ms: 100,
            lz4_compression_threshold: 4 * 1024,
            promotion_threshold: 16,
            bind_port: 9000,
            _padding: [0u8; 6],
        }
    }
}

/// Single-source-of-truth global configuration for the entire Laplace stack.
///
/// Sub-structs map 1-to-1 to the three main subsystems:
///
/// | Field    | Crate              | Controls                          |
/// |----------|--------------------|-----------------------------------|
/// | `axiom`  | `laplace-axiom`    | DPOR/Oracle verification engine   |
/// | `kraken` | `laplace-kraken`   | Load-simulation & network emulator|
/// | `probe`  | `laplace-probe`    | QUIC mesh sidecar & wire codec    |
///
/// **Memory layout (96 bytes, 8-byte aligned)**:
/// ```text
/// Offset  Size  Field
///  0      24    axiom   (AxiomConfig)
/// 24      40    kraken  (KrakenConfig)
/// 64      32    probe   (ProbeConfig)
/// ```
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Config",
        link = "LEP-0001-laplace-interfaces-global_ffi_config"
    )
)]
#[repr(C, align(8))]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LaplaceGlobalConfig {
    /// Axiom verification engine settings.
    pub axiom: AxiomConfig,
    /// Kraken load-simulation engine settings.
    pub kraken: KrakenConfig,
    /// Probe QUIC mesh-sidecar settings.
    pub probe: ProbeConfig,
}

const _: () = assert!(
    core::mem::size_of::<LaplaceGlobalConfig>() == 96,
    "LaplaceGlobalConfig must be exactly 96 bytes"
);

// ============================================================================
// ConfigSynchronizer — RCU hot-reload contract
// ============================================================================

/// Error variants for [`ConfigSynchronizer`] operations.
#[derive(Debug)]
pub enum ConfigSyncError {
    /// The IPC channel to the running daemon is closed or unreachable.
    ChannelClosed,
    /// The daemon received the config but rejected it (e.g. validation failure).
    Rejected(String),
}

impl core::fmt::Display for ConfigSyncError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ChannelClosed => write!(f, "IPC channel closed"),
            Self::Rejected(reason) => write!(f, "Config rejected by daemon: {reason}"),
        }
    }
}

/// RCU-based config synchroniser trait.
///
/// Implementors push a new [`LaplaceGlobalConfig`] snapshot into a running
/// Laplace daemon without stalling in-flight requests.
///
/// The reference implementation (`MockIpcSynchronizer` in `laplace-cli`) uses
/// a Unix-socket mock; production would use gRPC / shared memory.
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "10_Interfaces_Config",
        link = "LEP-0001-laplace-interfaces-global_ffi_config"
    )
)]
pub trait ConfigSynchronizer: Send + Sync {
    /// Atomically apply `config` to the running daemon.
    ///
    /// Returns `Ok(())` once the daemon has acknowledged the new snapshot.
    fn apply(&self, config: &LaplaceGlobalConfig) -> Result<(), ConfigSyncError>;

    /// Retrieve the currently active config snapshot from the daemon.
    fn current(&self) -> LaplaceGlobalConfig;
}

#[cfg(test)]
mod LaplaceConfig_tests {
    use super::*;

    #[test]
    fn LaplaceConfig_default_is_consistent() {
        let cfg = LaplaceConfig::default();
        assert_eq!(cfg.network.port, 4433);
        assert_eq!(cfg.network.max_connections, 1_000);
        assert_eq!(cfg.verification.max_virtual_users, 100);
        assert!(cfg.verification.deterministic);
        assert_eq!(cfg.resources.max_scheduler_threads, 8);
    }

    #[test]
    fn LaplaceConfig_serializes_to_json() {
        let cfg = LaplaceConfig::default();
        let json = serde_json::to_string(&cfg).expect("LaplaceConfig must serialize");
        assert!(json.contains("network"));
        assert!(json.contains("verification"));
        assert!(json.contains("resources"));
    }

    #[test]
    fn LaplaceConfig_round_trips_json() {
        let original = LaplaceConfig::default();
        let json = serde_json::to_string(&original).unwrap();
        let restored: LaplaceConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.network.port, original.network.port);
        assert_eq!(
            restored.verification.global_seed,
            original.verification.global_seed
        );
    }
}
