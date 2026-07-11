# laplace-probe-sdk

User SDK for the Laplace BYOC (Bring Your Own Code) verification system.

Applying `TrackedMutex` / `TrackedStdMutex` to existing code lets Classic DPOR
explore all execution interleavings to detect **lock-order inversion (AB-BA)**
deadlocks. It can detect them even when an actual deadlock does not occur.

---

## Quick start (3 steps)

### 1. Add dependencies

`Cargo.toml`:
```toml
[dev-dependencies]
laplace-probe-sdk = { path = "path/to/laplace-probe-sdk", features = ["verification"] }
```

### 2. Replace Mutex

`tokio::sync::Mutex`-based code:
```rust
// Before
use tokio::sync::Mutex;
struct SharedState { data: Mutex<Vec<i64>> }

// After
use laplace_probe_sdk::TrackedMutex;
struct SharedState { data: TrackedMutex<Vec<i64>> }
//                         ^ Rename only; the API is unchanged.
```

`std::sync::Mutex`-based code:
```rust
// Before
use std::sync::Mutex;

// After
use laplace_probe_sdk::TrackedStdMutex;
```

### 3. Write a test

```rust
use laplace_probe_sdk::{
    run_verification_from, set_probe_sender, set_probe_thread_id,
    ProbeEvent, ProbeSessionConfig, TrackedMutex,
};
use std::sync::{mpsc, Arc};

#[test]
fn verify_my_concurrent_code() {
    let (tx, rx) = mpsc::sync_channel::<ProbeEvent>(4096);

    let lock_a = Arc::new(TrackedMutex::new(0u64, "lock_a"));
    let lock_b = Arc::new(TrackedMutex::new(0u64, "lock_b"));

    let mut handles = Vec::new();

    for i in 0..2usize {
        let a = lock_a.clone();
        let b = lock_b.clone();
        let tx2 = tx.clone();
        handles.push(std::thread::spawn(move || {
            set_probe_sender(tx2);
            set_probe_thread_id(i as u64);
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all().build().unwrap();
            rt.block_on(async {
                let _g = a.lock().await;
                let _g = b.lock().await;
            });
        }));
    }

    drop(tx);
    for h in handles { h.join().unwrap(); }

    let events: Vec<ProbeEvent> = rx.into_iter().collect();

    let config = ProbeSessionConfig {
        write_ard: true,
        output_dir: "/tmp".to_string(),
        ..Default::default()
    };

    run_verification_from(&events, "my_test", &config).assert_clean();
}
```

---

## API reference

### TrackedMutex\<T\> (async)

`tokio::sync::Mutex<T>` wrapper.

```rust
use laplace_probe_sdk::TrackedMutex;

let mtx = TrackedMutex::new(value, "resource_name"); // &'static str required
let guard = mtx.lock().await;                         // sends LockAcquired on acquisition
// sends LockReleased automatically when the guard is dropped
```

### TrackedStdMutex\<T\> (sync)

`std::sync::Mutex<T>` wrapper.

```rust
use laplace_probe_sdk::TrackedStdMutex;

let mtx = TrackedStdMutex::new(value, "resource_name");
let guard = mtx.lock();  // synchronous, blocking
```

### run_verification_from

```rust
pub fn run_verification_from(
    events: &[ProbeEvent],
    test_name: &str,
    config: &ProbeSessionConfig,
) -> VerifyResult
```

### VerifyResult

```rust
result.assert_clean();  // panics if the result is not CLEAN
result.assert_bug();    // panics if the result is not BUG DETECTED
```

---

## Patching external libraries

Use `[patch.crates-io]` to replace a library's internal Mutex with TrackedMutex.

`Cargo.toml` (workspace root):
```toml
[patch.crates-io]
some-library = { path = "vendor/some-library-patched" }
```

Add this to the top of `vendor/some-library-patched/src/lib.rs`:
```rust
#[cfg(feature = "laplace")]
mod mutex_wrapper {
    use laplace_probe_sdk::TrackedMutex;
    // ... wrapper that uses TrackedMutex like std Mutex
}
```

See `examples/mobc-real/` and `examples/deadpool-hunt/` for detailed examples.

---

## Instrumentation coverage

| Synchronization primitive | Support |
|------------------|------|
| `tokio::sync::Mutex` | ✅ TrackedMutex |
| `std::sync::Mutex` | ✅ TrackedStdMutex |
| `tokio::sync::RwLock` | Planned |
| `tokio::sync::Semaphore` | Planned |

**Important**: uninstrumented code paths are not observed.
Events are collected only on paths where the library actually uses the patched
Mutex type.

---

## Running

```bash
# Single test (--test-threads=1 required for thread-local isolation)
cargo test -p my-crate my_test_name -- --test-threads=1 --nocapture

# All tests
./scripts/laplace-test.sh my-crate
```

**`--test-threads=1` is required.** The thread-local channels
(`PROBE_SENDER`, `PROBE_THREAD_ID`) must be isolated between tests so events do
not mix.
