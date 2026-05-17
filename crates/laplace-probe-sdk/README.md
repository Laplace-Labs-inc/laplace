# laplace-probe-sdk

Laplace BYOC (Bring Your Own Code) 검증 시스템의 사용자 SDK.

기존 코드에 `TrackedMutex` / `TrackedStdMutex`를 적용하면,
Ki-DPOR가 모든 실행 인터리빙을 탐색하여 **잠금 순서 역전(AB-BA)** 교착을 탐지한다.
실제 교착이 발생하지 않아도 탐지 가능하다.

---

## 빠른 시작 (3단계)

### 1단계: 의존성 추가

`Cargo.toml`:
```toml
[dev-dependencies]
laplace-probe-sdk = { path = "path/to/laplace-probe-sdk", features = ["verification"] }
```

### 2단계: Mutex 교체

`tokio::sync::Mutex` 기반 코드:
```rust
// Before
use tokio::sync::Mutex;
struct SharedState { data: Mutex<Vec<i64>> }

// After
use laplace_probe_sdk::TrackedMutex;
struct SharedState { data: TrackedMutex<Vec<i64>> }
//                         ^ 이름만 바꾼다. API는 동일하다.
```

`std::sync::Mutex` 기반 코드:
```rust
// Before
use std::sync::Mutex;

// After
use laplace_probe_sdk::TrackedStdMutex;
```

### 3단계: 테스트 작성

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

## API 참조

### TrackedMutex\<T\> (async)

`tokio::sync::Mutex<T>` 래퍼.

```rust
use laplace_probe_sdk::TrackedMutex;

let mtx = TrackedMutex::new(value, "resource_name"); // &'static str 필수
let guard = mtx.lock().await;                         // 획득 시 LockAcquired 전송
// guard drop 시 LockReleased 자동 전송
```

### TrackedStdMutex\<T\> (sync)

`std::sync::Mutex<T>` 래퍼.

```rust
use laplace_probe_sdk::TrackedStdMutex;

let mtx = TrackedStdMutex::new(value, "resource_name");
let guard = mtx.lock();  // 동기, 블로킹
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
result.assert_clean();  // CLEAN이 아니면 panic
result.assert_bug();    // BUG DETECTED가 아니면 panic
```

---

## 외부 라이브러리 패치

`[patch.crates-io]`를 사용하여 라이브러리 내부 Mutex를 TrackedMutex로 교체할 수 있다.

`Cargo.toml` (workspace root):
```toml
[patch.crates-io]
some-library = { path = "vendor/some-library-patched" }
```

`vendor/some-library-patched/src/lib.rs` 상단에 추가:
```rust
#[cfg(feature = "laplace")]
mod mutex_wrapper {
    use laplace_probe_sdk::TrackedMutex;
    // ... TrackedMutex를 std Mutex처럼 사용하는 래퍼
}
```

상세 예시: `examples/mobc-real/`, `examples/deadpool-hunt/` 참조.

---

## 계측 범위

| 동기화 primitive | 지원 |
|------------------|------|
| `tokio::sync::Mutex` | ✅ TrackedMutex |
| `std::sync::Mutex` | ✅ TrackedStdMutex |
| `tokio::sync::RwLock` | 향후 |
| `tokio::sync::Semaphore` | 향후 |

**중요**: 계측하지 않은 코드 경로는 관측되지 않는다.
라이브러리가 패치된 Mutex 타입을 실제로 사용하는 경로에서만 이벤트가 수집된다.

---

## 실행 방법

```bash
# 단일 테스트 (--test-threads=1 필수 — thread-local 격리)
cargo test -p my-crate my_test_name -- --test-threads=1 --nocapture

# 전체 테스트
./scripts/laplace-test.sh my-crate
```

**`--test-threads=1`은 필수**다. thread-local 채널(`PROBE_SENDER`, `PROBE_THREAD_ID`)이
테스트 간 격리되어야 이벤트가 혼합되지 않는다.
