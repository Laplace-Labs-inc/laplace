# pingora-pool 0.8.1 hunt scope

This example is Route A of the HUNT-PG1 experiment. It captures the real
vendored `ConnectionPool::idle_timeout` protocol through
`#[laplace_sdk::verify(tasks)]`; it does not hand-model the protocol.

## `UNSUPPORTED_ASYNC_SURFACES` scope manifest

The modeled target is `idle_timeout`, whose four `tokio::select!` branches are
all Laplace-visible async primitives: `oneshot`, `Notify`, `watch`, and
`sleep`. The `biased` choice is retained from upstream. S1 passes `None` for
the timeout, so the sleep branch is conditionally disabled and the native
capture contains no timer event. The sleep branch itself is exercised in the
Route B engine test (`s1b`, `Some(5ms)`) under the engine's virtual clock;
only the capture route keeps `None`, because timer visibility is a capture
boundary (see below).

The following surfaces remain intentionally off-model and are not claims of
coverage:

- `parking_lot::{Mutex,RwLock}`: every observed lock scope is lexical inside a
  synchronous function and crosses no `.await`; async interleaving therefore
  does not split those scopes in this model.
- `crossbeam_queue::ArrayQueue`: retained as a native queue and not represented
  as an async event source.
- `lru::LruCache`, `thread_local::ThreadLocal`, and `RefCell`: retained as the
  upstream LRU implementation; their thread-local shard cross-thread race
  class is outside this single deterministic async model.
- `AtomicBool drain`: retained as native state and not represented as a model
  event.
- `idle_poll`: excluded because its read branch consumes a real
  `tokio::io::AsyncRead` through `OwnedMutexGuard`; the hunt does not pretend
  that I/O is modeled.

Route A installs the public async probe hooks only. It does not install the
private engine `AsyncTimerHook`; timer visibility is therefore a capture
boundary, and the S1 timer absence must not be read as proof that timer
interleavings were explored. Route B uses `AsyncLiveSource` and its virtual
clock for the direct engine exploration.

## S5 dynamic-spawn topology

Route B also contains three direct-engine compositions (N1-N3) that mirror the
real consumer topology: after `put`, an async idle watcher is dynamically
spawned, then `get`, replacement/eviction, or close notification races with
that watcher. N1 repeats the reuse cycle twice. The compositions call the
vendored `ConnectionPool::idle_timeout` directly; `idle_poll` remains excluded
because its read branch is real I/O. Each composition is bounded to the model's
eight cumulative tasks and checks a fixed-seed 20/20 signature.

The pingora-core 0.8.1 source was rechecked from crates.io: production
`connectors/mod.rs:271-278` calls `rt.spawn(async { pool.idle_poll(...).await })`
and discards the native handle. There is no production `.abort()` on this idle
housekeeping path; the only `.abort()` hits in the crate are two `#[cfg(test)]`
mock UDS server handles (`connectors/http/mod.rs:473,513`). Therefore abort is
not mirrored or claimed as a pingora real-path result in S5.

## S7 Route A customer-surface dynamic-spawn re-hunt

S7 adds three native customer-surface mirrors beside S1:
`pingora_pool_s7_n1_reuse_cycle`, `pingora_pool_s7_n2_eviction_vs_pickup`, and
`pingora_pool_s7_n3_close_notify_vs_pickup`. They call the vendored
`ConnectionPool::idle_timeout` through `#[laplace_sdk::verify(tasks)]`; the
`tokio::spawn` tokens in the example are rewritten to
`laplace_sdk::rt::spawn_task` by the tasks-mode macro. Their Route B counterparts
are `s5_n1_reuse_cycle`, `s5_n2_eviction_vs_pickup`, and
`s5_n3_close_notify_vs_pickup` in `crates/laplace-cli/tests/pingora_hunt_engine.rs`.
All three Route A mirrors pass `timeout=None`, so timer capture is intentionally
absent. Every watcher is ended by pickup, eviction, or close signaling; N1 uses
an adapter-only handle await to keep its two pickup-terminated dynamic watchers
inside the one-shot native capture.

The two independent captures on 2026-07-13 were byte-identical:

| target | events | SHA-256 |
|---|---:|---|
| N1′ reuse-cycle | 24 | `31d7c279f3727a53f6970b083675514965298335ee76eb9d6f5d3e9ae3878950` |
| N2′ eviction-vs-pickup | 24 | `30d91ae72dc5d79b195296edb23d8a33c9ec902b11b03eca4d72ab02d02cba60` |
| N3′ close-notify-vs-pickup | 23 | `a59f29a55089ac4d8c14fc0525590203203c10e2aa9cf3c0e4b0fce5cb62ffb9` |

Reproduce the capture and customer-path replay from the repository root:

```text
cd open
LAPLACE_VERIFY_EVENTS_DIR=/tmp/pingora-hunt-s7 \
  cargo test -p pingora-hunt -- --test-threads=1
cd ..
cargo test -p laplace-cli --test pingora_hunt_replay -- --nocapture
laplace axiom verify --model-events crates/laplace-cli/tests/fixtures/pingora_hunt --no-cache
```

The combined customer replay is intentionally not an eviction-class support
claim: N1′ and N3′ are `clean`, while N2′ is `UNSUPPORTED` with
`oneshot channel 14 endpoint cannot move from creation task` and exit 2. This
is the existing `channel-endpoint-move` tier-2 boundary observed on the real
eviction topology, where the eviction signal's oneshot endpoint is created in
one task and observed from the dynamic child. Route B remains the direct-engine
path for this eviction class; Route A capture does not erase the tier-2 limit.
