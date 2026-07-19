# bb8 0.9.1 BB2 hunt scope

This example is Route A for HUNT-BB2.1. The three `verify(tasks)` compositions
capture the vendored `bb8_async_patched::Pool` itself; the route does not replace
the pool protocol with a hand-written model.

## Scenarios

- `bb8_hunt2_w_return` is the cancellation-free control. It builds a pool with
  `Builder::build().await`, holds the initial connection, and races a normal
  return against a second getter.
- `bb8_hunt2_w_broken` marks the held connection broken before returning it.
  After ASYNC-SPAWN S3 de-patching, native capture must contain a dynamic
  `TaskSpawned` marker for replenishment; tier-2 replay must reject it with
  `dynamic-spawn-replay` (exit 2).
- `bb8_hunt2_w_cancel` parks a real pool getter on bb8's `Notify`, then cancels
  that getter through a `tokio::select!` loser before the owner returns its
  connection. Native capture is expected to terminate; tier-2 linear replay is
  expected to classify the waiter-drop evidence as `UNSUPPORTED` (exit 2).

## Scope manifest

The modeled async surface is bb8 0.9.1's `Arc<Notify>` wait/release path:
`notified().await`, `notify_one()`, the connection-timeout wrapper, and the
`build().await` initialization path. All three scenarios set `max_lifetime(None)` and
`idle_timeout(None)`, so the conditional reaper is not started.

The following boundaries are explicit and are not claims of coverage:

- bb8's `crate::lock::Mutex` is a synchronous `parking_lot` (or std) lock. The
  observed lock scopes are lexical and do not cross `.await`, so they remain
  atomic at this async interleaving layer.
- `AtomicU64` statistics, `VecDeque` bookkeeping, `Instant`, and
  `futures_util::FuturesUnordered` are retained native implementation details;
  `FuturesUnordered` is a combinator, not an independently modeled primitive.
- The reaper's native `interval_at`/`tokio::spawn` path and `build_unchecked()`
  dynamic-start path are excluded. The modeled `spawn_replenishing_approvals`
  boundary is routed through `laplace_model_rt::spawn_task` so S3 can observe its
  dynamic task; the upstream approval and replenishment control flow is kept.
- Route A installs the public async probe hooks but no private
  `AsyncTimerHook`; timer deadlines therefore do not appear as captured events.
  The scenarios terminate through connection/notification signals before the
  timeout is the deciding event.
- Real I/O, external threads, and cross-thread runtime sharding are outside this
  composition. No reaper or `build_unchecked` schedule is reported as covered.

The upstream MIT license is preserved in `open/vendor/bb8-async-patched/LICENSE`;
the local substitutions and their limitations are listed in that crate's
`NOTICE`.
