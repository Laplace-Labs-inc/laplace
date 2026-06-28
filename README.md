<div align="center">

<img src="https://raw.githubusercontent.com/Laplace-Labs-inc/laplace-web/main/astro/public/images/Laplace_labs.svg" alt="Laplace Labs" width="400" />

<br><br>

**Deterministic concurrency verification for production Rust systems.**

<br>

[![Version](https://img.shields.io/badge/version-0.1.0--alpha--1-orange)](Cargo.toml)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)
[![CI](https://github.com/Laplace-Labs-inc/laplace/actions/workflows/ci.yml/badge.svg)](https://github.com/Laplace-Labs-inc/laplace/actions/workflows/ci.yml)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-dea584?logo=rust)](https://www.rust-lang.org/)

</div>

---

Laplace detects deadlocks, data races, and starvation in concurrent Rust code — **deterministically**, not probabilistically. It uses Ki-DPOR + MCR to exhaustively explore the state space that probabilistic testing can never reliably reach.

**[Documentation](https://laplace-labs.com/docs) · [Getting Started](https://laplace-labs.com/docs/getting-started) · [Bug Reference](https://laplace-labs.com/docs/bug-reference)**

---

## Installation

Prebuilt, signed binaries — no Rust toolchain required.

**Linux / macOS**

```sh
curl -fsSL https://install.laplace-labs.com | sh
```

**Windows (PowerShell)**

```powershell
irm https://install.laplace-labs.com/install.ps1 | iex
```

The installer picks the right binary for your platform, verifies its SHA-256
checksum, and installs `laplace` to `~/.laplace/bin`. Linux binaries are static
(musl), so they run on any distribution regardless of the host glibc version.

Prefer a manual download? Grab an archive from the
[Releases page](https://github.com/Laplace-Labs-inc/laplace/releases/latest)
(`laplace-cli-<target>.tar.xz` / `.zip`), verify its `.sha256`, and put
`laplace` on your `PATH`. From source: `cargo install laplace-cli` (slower).

```sh
# Authenticate (free tier available)
laplace auth activate <license-key>
```

## Quick Start

Add the SDK to your project:

```toml
# Cargo.toml
[dependencies]
laplace-sdk = "0.1.0-alpha-1"
```

Annotate your test harness:

```rust
use laplace_sdk::prelude::*;

#[axiom_harness]
async fn my_concurrent_test() {
    // Laplace exhaustively explores all interleavings
    let mutex = TrackedMutex::new(0u64);
    // ...
}
```

Run verification:

```bash
laplace axiom run --harness my_concurrent_test
```

No license required for mock verification:

```bash
laplace axiom mock-verify --scenario deadlock   # must exit 1
laplace axiom mock-verify --scenario clean      # must exit 0
```

---

## How It Works

The `laplace` CLI binary contains the Ki-DPOR engine. Your code only depends on the thin SDK — no engine source is compiled into your binary.

```
Your crate
└── laplace-sdk          (this repo, ~thin client)
    └── #[axiom_harness] (instrumentation only)

laplace binary           (installed separately)
└── Ki-DPOR engine       (closed, exhaustive verification)
└── Kraken load engine   (deterministic chaos)
└── Probe mesh           (QUIC/eBPF observation)
```

This follows the same pattern as `cargo miri`, `tokio-console`, and Valgrind: **the tool is the runner, your code is the spec**.

---

## Engines

| Engine | Description | Tier |
| :--- | :--- | :--- |
| **Axiom** | Ki-DPOR + MCR exhaustive verification — deadlock, race, starvation | FREE+ |
| **Kraken** | ChaCha8 deterministic RNG + scenario DSL chaos load simulator | PRO+ |
| **Probe** | QUIC/eBPF ultra-low-latency semantic observation mesh | ULTRA+ |

---

## Public Crates

This repository contains the open SDK tier. The engine is distributed as a CLI binary.

| Crate | Status | Description |
| :--- | :--- | :--- |
| `laplace-interfaces` | published | ABI/FFI types (`#[repr(C)]`) |
| `laplace-macro` | published | Proc-macros (`#[axiom_harness]`) |
| `laplace-probe-common` | published | `RawProbeEvent` ABI |
| `laplace-sdk` | alpha-2 | Master re-export — user entry point |
| `laplace-probe-sdk` | alpha-2 | `TrackedMutex`, BYOC integration |
| `laplace-harness` | alpha-2 | Built-in verification scenarios |

---

## Known Concurrency Bugs

Bugs found in production Rust libraries using Laplace:

| Library | Version | Bug | Severity |
| :--- | :--- | :--- | :--- |
| **dashmap** | 6.1.0 | AB-BA deadlock (shard RwLock) | Critical |
| **deadpool** | 0.10.0 | Semaphore permit mismatch | Critical |
| **deadpool** | 0.13.0 | Mutex + async boundary crossing | High |
| **mobc** | 0.9.0 | async Mutex deadlock | Critical |
| **bb8** | latest | Pool state counter drift | High |
| **r2d2** | latest | Condvar notify omission | Critical |
| **tokio** | 1.x | Mutex self-deadlock | High |
| **crossbeam** | latest | `select!` non-deterministic ordering | High |
| **deadpool-runtime** | 0.1.x | Dispatch context leak | Low |

Full reference: [laplace-labs.com/docs/bug-reference](https://laplace-labs.com/docs/bug-reference)

---

## Documentation

Full documentation is available at **[laplace-labs.com/docs](https://laplace-labs.com/docs)**.

| Topic | Link |
| :--- | :--- |
| Getting Started | [laplace-labs.com/docs/getting-started](https://laplace-labs.com/docs/getting-started) |
| CLI Reference | [laplace-labs.com/docs/cli-reference](https://laplace-labs.com/docs/cli-reference) |
| API Reference | [laplace-labs.com/docs/api-reference](https://laplace-labs.com/docs/api-reference) |
| Bug Reference | [laplace-labs.com/docs/bug-reference](https://laplace-labs.com/docs/bug-reference) |
| Roadmap | [laplace-labs.com/docs/roadmap](https://laplace-labs.com/docs/roadmap) |

---

## Pricing

| Tier | Axiom | Kraken | Probe | max_depth |
| :--- | :---: | :---: | :---: | :---: |
| FREE | ✓ | | | 500 |
| PLUS | ✓ | | | 10,000 |
| PRO | ✓ | ✓ | | ∞ |
| ULTRA | ✓ | ✓ | ✓ | ∞ |
| ENTERPRISE | ✓ | ✓ | ✓ | ∞ |

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

Bugs and security issues: `security@laplace-labs.com`  
Enterprise and sales: `enterprise@laplace-labs.com`

---

## License

Apache-2.0 — see [LICENSE](LICENSE).

Engine source (`laplace-ki-dpor`, `laplace-kraken`, `laplace-probe`) is proprietary and distributed as compiled binaries only. See [laplace-labs.com/docs/open-core](https://laplace-labs.com/docs/open-core) for the open-core boundary specification.

---

<div align="center">

**Laplace Labs** — *Observe. Prove. Heal.*

</div>
