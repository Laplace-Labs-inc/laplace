# Non-vendored async LeasePool PoC

This example is a customer-owned, capacity-one lease/idle-pool protocol. It
does not copy, vendor, fork, or patch a third-party pool. The customer protocol
is intentionally changed at explicit seams: `#[laplace_sdk::verify(tasks)]`
and the public `laplace_sdk::rt` types used in `src/lib.rs`.

## Two routes

- Route A is the public native-capture path. The fixed control registers five
  tasks (manager, watcher, waiter, owner, and supervisor), runs the shared
  `LeasePool` methods natively, and uses `TaskSet` join handles.
- Route B is the private direct-engine path. The integration test composes the
  same `LeasePool` methods through `AsyncLiveSource` and explores schedules
  directly. Its test-only `fault-fixture` feature suppresses exactly one real
  `release()` wake; it does not inject ProbeEvents, verdicts, witnesses, or
  artificial yields.

Both routes use `ModelAsyncMutex`, `ModelAsyncNotify`, bounded `mpsc`, and
`time::timeout`. The release transition drops the mutex guard before notifying
the waiter and sending the bounded manager command. No normal path holds a
mutex guard across `.await`, retains a permanent sender after shutdown, or
uses `mem::forget`, `pending()`, or arbitrary sleep/yield to manufacture a
witness.

## Honest boundary

The timer claim is limited to Route B's private virtual-clock semantics for
the shared `time::timeout` branch. The measured S2 witness included a
`ClockAdvance` wake; the terminal evidence returned after the bounded S1/S3
Clean runs did not, so those Clean results do not claim observed expiry
interleavings. Route A installs public async capture hooks only; it has no
timer hook, so the absence of timer events in its envelope is not evidence
that timer interleavings were explored.

This PoC does not claim coverage for real I/O, a multi-thread Tokio runtime,
thread-local sharding, select-loser cancellation replay, channel endpoint
moves, multi-receiver channel topology, or more than eight model tasks. The
protocol itself is not a third-party customer crate: “non-vendored” means no
third-party fork or patch, not “customer code was analyzed without an explicit
seam or annotation.”
