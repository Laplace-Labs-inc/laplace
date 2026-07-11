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
