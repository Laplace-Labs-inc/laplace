// SPDX-License-Identifier: Apache-2.0
//! Process-local engine hooks and deterministic model resource-id allocation.
//!
//! The engine installs a [`SpawnHook`] and/or [`LockHook`] to take control of a
//! model run; with no hook installed the seams in [`crate::spawn`],
//! [`crate::mutex`], and [`crate::rwlock`] delegate to the standard library.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use crate::spawn::JoinToken;

static SPAWN_HOOK: OnceLock<StdMutex<Option<Arc<dyn SpawnHook>>>> = OnceLock::new();
static LOCK_HOOK: OnceLock<StdMutex<Option<Arc<dyn LockHook>>>> = OnceLock::new();
static NEXT_LOCK_RESOURCE_ID: AtomicU64 = AtomicU64::new(1);
static ASYNC_LOCK_HOOK: OnceLock<StdMutex<Option<Arc<dyn AsyncLockHook>>>> = OnceLock::new();
static ASYNC_NOTIFY_HOOK: OnceLock<StdMutex<Option<Arc<dyn AsyncNotifyHook>>>> = OnceLock::new();
static ASYNC_CHANNEL_HOOK: OnceLock<StdMutex<Option<Arc<dyn AsyncChannelHook>>>> = OnceLock::new();
static ASYNC_TIMER_HOOK: OnceLock<StdMutex<Option<Arc<dyn AsyncTimerHook>>>> = OnceLock::new();
/// Shared across the whole async model family (Mutex, `RwLock`, Semaphore,
/// Notify, the `mpsc`/oneshot/watch channels, and the timer shadows) so
/// resource ids never collide across the family, even when several are mixed
/// in one model run.
static NEXT_ASYNC_LOCK_RESOURCE_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_ASYNC_LOCK_WAITER_ID: AtomicU64 = AtomicU64::new(1);
/// Dedicated gate for [`crate::laplace_select`]'s `biased;` polling. Deliberately
/// its own switch rather than inferred from any hook's presence — a model run
/// installs the timer/lock hooks *and* flips this flag; a plain user build
/// (even one that happens to link an engine hook for unrelated reasons) must
/// not silently lose tokio's stock random polling fairness.
static DETERMINISTIC_SELECT: AtomicBool = AtomicBool::new(false);

/// Engine-installed surface for creating one controlled model thread.
pub trait SpawnHook: Send + Sync {
    /// Creates one model thread under engine control.
    fn spawn(&self, f: Box<dyn FnOnce() + Send + 'static>) -> JoinToken;
}

/// Engine- or probe-installed surface for model mutex boundaries.
pub trait LockHook: Send + Sync {
    /// Reports a model mutex acquisition boundary.
    fn acquire(&self, resource: u64);

    /// Reports a model mutex release boundary.
    fn release(&self, resource: u64);
}

/// Acquisition mode for an async lock-family boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsyncAcquireKind {
    /// [`crate::ModelAsyncMutex::lock`]/`try_lock`.
    Mutex,
    /// [`crate::ModelAsyncRwLock::read`]/`try_read`.
    RwRead,
    /// [`crate::ModelAsyncRwLock::write`]/`try_write`.
    RwWrite,
    /// `n` permits (`acquire`/`try_acquire` = 1, `acquire_many`/
    /// `try_acquire_many(n)` = `n`).
    SemaphorePermits(u32),
}

/// Engine- or probe-installed surface for model async lock-family boundaries.
///
/// Mirrors [`LockHook`] but for the `tokio::sync::{Mutex,RwLock,Semaphore}`-
/// compatible async seam ([`crate::ModelAsyncMutex`], [`crate::ModelAsyncRwLock`],
/// [`crate::ModelAsyncSemaphore`]), which additionally distinguishes a queued
/// waiter (a live, unpolled acquisition future) from an acquired guard/permit,
/// and an acquisition's mode via [`AsyncAcquireKind`].
/// [`crate::ModelAsyncNotify`] is a distinct wait/wake vocabulary, not an
/// acquisition — see [`AsyncNotifyHook`].
pub trait AsyncLockHook: Send + Sync {
    /// Reports that an acquisition future's first poll found contention and
    /// queued behind the current holder(s).
    fn requested(&self, resource: u64, waiter: u64, kind: AsyncAcquireKind);

    /// Reports a guard/permit acquisition, either immediately (uncontended)
    /// or by resolving a previously queued waiter.
    fn acquired(&self, resource: u64, waiter: u64, kind: AsyncAcquireKind);

    /// Reports a model async lock-family release boundary.
    fn released(&self, resource: u64, waiter: u64, kind: AsyncAcquireKind);

    /// Reports that a queued-but-unacquired acquisition future was dropped
    /// (cancelled) before it resolved.
    fn waiter_dropped(&self, resource: u64, waiter: u64);

    /// Reported once per semaphore, lazily at its first observed boundary,
    /// before that boundary's event — carries the initial permit capacity.
    fn semaphore_created(&self, resource: u64, permits: usize);

    /// `Semaphore::add_permits(n)` capacity increase.
    fn permits_added(&self, resource: u64, n: usize);
}

/// Engine- or probe-installed surface for [`crate::ModelAsyncNotify`]
/// wait/wake boundaries.
pub trait AsyncNotifyHook: Send + Sync {
    /// `notified()` future's first poll found no stored permit and queued.
    fn wait_requested(&self, resource: u64, waiter: u64);

    /// `notified()` future resolved (immediately via a stored permit, or by
    /// a wake).
    fn wait_resolved(&self, resource: u64, waiter: u64);

    /// `notify_one()` boundary.
    fn notify_one(&self, resource: u64);

    /// `notify_waiters()` boundary.
    fn notify_waiters(&self, resource: u64);

    /// Reports that a queued-but-unresolved `notified()` future was dropped
    /// before it resolved.
    fn waiter_dropped(&self, resource: u64, waiter: u64);
}

/// Which channel flavor a model channel resource is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsyncChannelKind {
    MpscBounded { capacity: usize },
    MpscUnbounded,
    Oneshot,
    Watch,
}

/// Which side of a channel an endpoint event concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsyncChannelSide {
    Sender,
    Receiver,
}

/// Operation classification for channel op events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsyncChannelOp {
    Send,
    Recv,
    Changed,
}

/// Terminal outcome of a channel op event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsyncChannelOutcome {
    Ok,
    Closed,
    Empty,
    Full,
}

/// Engine- or probe-installed surface for model `tokio::sync` channel
/// (`mpsc`/oneshot/watch) boundaries.
///
/// Mirrors [`AsyncLockHook`]/[`AsyncNotifyHook`] but for the channel family:
/// `channel`/`op`/`endpoint` events replace the lock family's
/// `resource`/`waiter` acquisition vocabulary, since a channel's boundaries
/// are sends, receives, and endpoint lifecycle rather than acquire/release.
pub trait AsyncChannelHook: Send + Sync {
    /// Reported once per channel, at construction, carrying its flavor.
    fn channel_created(&self, channel: u64, kind: AsyncChannelKind);

    /// Reports that an awaitable op's first poll found no immediate result
    /// and queued (mirrors [`AsyncLockHook::requested`]).
    fn op_requested(&self, channel: u64, op: u64, kind: AsyncChannelOp);

    /// Reports an op's terminal outcome, either immediately (uncontended or
    /// a synchronous try-op) or by resolving a previously queued op.
    fn op_resolved(
        &self,
        channel: u64,
        op: u64,
        kind: AsyncChannelOp,
        outcome: AsyncChannelOutcome,
    );

    /// Reports that a queued-but-unresolved awaitable op was dropped
    /// (cancelled) before it resolved.
    fn op_dropped(&self, channel: u64, op: u64);

    /// Reports a sender/receiver endpoint handle being cloned (or, for
    /// `watch`, a receiver being created via `subscribe`).
    fn endpoint_cloned(&self, channel: u64, side: AsyncChannelSide);

    /// Reports a sender/receiver endpoint handle being dropped.
    fn endpoint_dropped(&self, channel: u64, side: AsyncChannelSide);

    /// Reports a receiver's `close()` boundary.
    fn channel_closed(&self, channel: u64);
}

/// Deterministic virtual-clock seam for the model time shadows.
pub trait AsyncTimerHook: Send + Sync {
    /// Current virtual time in nanoseconds.
    fn now_nanos(&self) -> u64;
    /// Registers interest in waking once virtual time reaches
    /// `deadline_nanos`. Called on every pending poll; re-registration of
    /// the same `(timer, deadline_nanos)` must be a cheap no-op.
    fn register(&self, timer: u64, deadline_nanos: u64);
    /// A pending timer future was dropped before its deadline.
    fn timer_dropped(&self, timer: u64);
}

/// Installs or replaces the process-local spawn hook.
///
/// # Panics
///
/// Panics if the internal hook registry mutex is poisoned.
pub fn install_spawn_hook(hook: Arc<dyn SpawnHook>) {
    let slot = SPAWN_HOOK.get_or_init(|| StdMutex::new(None));
    *slot.lock().expect("spawn hook lock poisoned") = Some(hook);
}

/// Clears the process-local spawn hook.
///
/// # Panics
///
/// Panics if the internal hook registry mutex is poisoned.
pub fn clear_spawn_hook() {
    if let Some(slot) = SPAWN_HOOK.get() {
        *slot.lock().expect("spawn hook lock poisoned") = None;
    }
}

pub(crate) fn spawn_hook() -> Option<Arc<dyn SpawnHook>> {
    SPAWN_HOOK
        .get()
        .and_then(|slot| slot.lock().expect("spawn hook lock poisoned").clone())
}

/// Installs or replaces the process-local lock hook.
///
/// # Panics
///
/// Panics if the internal hook registry mutex is poisoned.
pub fn install_lock_hook(hook: Arc<dyn LockHook>) {
    let slot = LOCK_HOOK.get_or_init(|| StdMutex::new(None));
    *slot.lock().expect("lock hook lock poisoned") = Some(hook);
}

/// Clears the process-local lock hook.
///
/// # Panics
///
/// Panics if the internal hook registry mutex is poisoned.
pub fn clear_lock_hook() {
    if let Some(slot) = LOCK_HOOK.get() {
        *slot.lock().expect("lock hook lock poisoned") = None;
    }
}

pub(crate) fn lock_hook() -> Option<Arc<dyn LockHook>> {
    LOCK_HOOK
        .get()
        .and_then(|slot| slot.lock().expect("lock hook lock poisoned").clone())
}

/// Resets deterministic model mutex id allocation for controlled re-execution.
///
/// This is separate from hook installation so free-tier code keeps process-wide
/// distinct resource ids, while the private engine can make each reset replay
/// the same two-resource program shape.
#[doc(hidden)]
pub fn reset_model_mutex_ids_for_model() {
    NEXT_LOCK_RESOURCE_ID.store(1, Ordering::SeqCst);
}

/// Allocates the next distinct process-local model resource id.
pub(crate) fn next_lock_resource_id() -> u64 {
    NEXT_LOCK_RESOURCE_ID.fetch_add(1, Ordering::SeqCst)
}

/// Installs or replaces the process-local async lock hook.
///
/// # Panics
///
/// Panics if the internal hook registry mutex is poisoned.
pub fn install_async_lock_hook(hook: Arc<dyn AsyncLockHook>) {
    let slot = ASYNC_LOCK_HOOK.get_or_init(|| StdMutex::new(None));
    *slot.lock().expect("async lock hook lock poisoned") = Some(hook);
}

/// Clears the process-local async lock hook.
///
/// # Panics
///
/// Panics if the internal hook registry mutex is poisoned.
pub fn clear_async_lock_hook() {
    if let Some(slot) = ASYNC_LOCK_HOOK.get() {
        *slot.lock().expect("async lock hook lock poisoned") = None;
    }
}

pub(crate) fn async_lock_hook() -> Option<Arc<dyn AsyncLockHook>> {
    ASYNC_LOCK_HOOK
        .get()
        .and_then(|slot| slot.lock().expect("async lock hook lock poisoned").clone())
}

/// Installs or replaces the process-local async notify hook.
///
/// # Panics
///
/// Panics if the internal hook registry mutex is poisoned.
pub fn install_async_notify_hook(hook: Arc<dyn AsyncNotifyHook>) {
    let slot = ASYNC_NOTIFY_HOOK.get_or_init(|| StdMutex::new(None));
    *slot.lock().expect("async notify hook lock poisoned") = Some(hook);
}

/// Clears the process-local async notify hook.
///
/// # Panics
///
/// Panics if the internal hook registry mutex is poisoned.
pub fn clear_async_notify_hook() {
    if let Some(slot) = ASYNC_NOTIFY_HOOK.get() {
        *slot.lock().expect("async notify hook lock poisoned") = None;
    }
}

pub(crate) fn async_notify_hook() -> Option<Arc<dyn AsyncNotifyHook>> {
    ASYNC_NOTIFY_HOOK.get().and_then(|slot| {
        slot.lock()
            .expect("async notify hook lock poisoned")
            .clone()
    })
}

/// Installs or replaces the process-local async channel hook.
///
/// # Panics
///
/// Panics if the internal hook registry mutex is poisoned.
pub fn install_async_channel_hook(hook: Arc<dyn AsyncChannelHook>) {
    let slot = ASYNC_CHANNEL_HOOK.get_or_init(|| StdMutex::new(None));
    *slot.lock().expect("async channel hook lock poisoned") = Some(hook);
}

/// Clears the process-local async channel hook.
///
/// # Panics
///
/// Panics if the internal hook registry mutex is poisoned.
pub fn clear_async_channel_hook() {
    if let Some(slot) = ASYNC_CHANNEL_HOOK.get() {
        *slot.lock().expect("async channel hook lock poisoned") = None;
    }
}

pub(crate) fn async_channel_hook() -> Option<Arc<dyn AsyncChannelHook>> {
    ASYNC_CHANNEL_HOOK.get().and_then(|slot| {
        slot.lock()
            .expect("async channel hook lock poisoned")
            .clone()
    })
}

/// Installs or replaces the process-local async timer hook.
///
/// # Panics
///
/// Panics if the internal hook registry mutex is poisoned.
pub fn install_async_timer_hook(hook: Arc<dyn AsyncTimerHook>) {
    let slot = ASYNC_TIMER_HOOK.get_or_init(|| StdMutex::new(None));
    *slot.lock().expect("async timer hook lock poisoned") = Some(hook);
}

/// Clears the process-local async timer hook.
///
/// # Panics
///
/// Panics if the internal hook registry mutex is poisoned.
pub fn clear_async_timer_hook() {
    if let Some(slot) = ASYNC_TIMER_HOOK.get() {
        *slot.lock().expect("async timer hook lock poisoned") = None;
    }
}

pub(crate) fn async_timer_hook() -> Option<Arc<dyn AsyncTimerHook>> {
    ASYNC_TIMER_HOOK
        .get()
        .and_then(|slot| slot.lock().expect("async timer hook lock poisoned").clone())
}

/// Enables/disables deterministic (`biased;`) branch polling for
/// [`crate::laplace_select`]. Installed by the engine for a model run; the
/// default (`false`) leaves user builds on tokio's stock random polling.
pub fn set_deterministic_select(enabled: bool) {
    DETERMINISTIC_SELECT.store(enabled, Ordering::SeqCst);
}

/// Whether [`crate::laplace_select`] currently forces `biased;` polling.
pub fn deterministic_select_enabled() -> bool {
    DETERMINISTIC_SELECT.load(Ordering::SeqCst)
}

/// Resets deterministic model async model-family resource-id and waiter-id
/// allocation for controlled re-execution.
///
/// This is separate from hook installation so free-tier code keeps
/// process-wide distinct resource/waiter ids, while the private engine can
/// make each reset replay the same resource/waiter shape. Shared by all
/// async model-family primitives (Mutex, RwLock, Semaphore, Notify, and the
/// `mpsc`/oneshot/watch channels) since they share one id space.
#[doc(hidden)]
pub fn reset_model_async_ids_for_model() {
    NEXT_ASYNC_LOCK_RESOURCE_ID.store(1, Ordering::SeqCst);
    NEXT_ASYNC_LOCK_WAITER_ID.store(1, Ordering::SeqCst);
}

/// Allocates the next distinct process-local model async model-family
/// resource id (also used as a channel id by the `mpsc`/oneshot/watch
/// channel seams).
pub(crate) fn next_async_lock_resource_id() -> u64 {
    NEXT_ASYNC_LOCK_RESOURCE_ID.fetch_add(1, Ordering::SeqCst)
}

/// Allocates the next distinct process-local model async model-family waiter
/// id, one per acquisition-future call (not per task — see
/// [`crate::ModelAsyncLock`]). Also used as a channel op id by the
/// `mpsc`/oneshot/watch channel seams, one per send/recv/changed call.
pub(crate) fn next_async_lock_waiter_id() -> u64 {
    NEXT_ASYNC_LOCK_WAITER_ID.fetch_add(1, Ordering::SeqCst)
}

/// Lazily-assigned resource id for `const fn` constructors (`const_new`).
///
/// `0` is the unassigned sentinel; real ids run from 1 (mirrors
/// [`next_async_lock_resource_id`]). Assignment happens on first
/// [`AsyncResourceId::get`] call rather than at construction, so this type
/// itself can be built in a `const` context.
pub(crate) struct AsyncResourceId(AtomicU64);

impl AsyncResourceId {
    /// Allocates a resource id immediately (mirrors the existing eager
    /// constructors' behavior — `new`, not `const_new`).
    pub(crate) fn new_eager() -> Self {
        Self(AtomicU64::new(next_async_lock_resource_id()))
    }

    /// Defers allocation until the first [`AsyncResourceId::get`] call, so
    /// `const fn` constructors can build this at compile time.
    pub(crate) const fn new_lazy() -> Self {
        Self(AtomicU64::new(0))
    }

    /// Returns this resource's id, allocating it on first use.
    ///
    /// Concurrent first calls on free-tier (multi-threaded, no engine hook)
    /// code may race: both see the unassigned sentinel and both allocate an
    /// id, but only one wins the `compare_exchange` and the loser adopts the
    /// winner's id. One id number leaking unused is harmless; the
    /// deterministic engine drives model runs single-threaded, so this race
    /// never occurs there.
    pub(crate) fn get(&self) -> u64 {
        let current = self.0.load(Ordering::SeqCst);
        if current != 0 {
            return current;
        }
        let allocated = next_async_lock_resource_id();
        match self
            .0
            .compare_exchange(0, allocated, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(_) => allocated,
            Err(existing) => existing,
        }
    }
}
