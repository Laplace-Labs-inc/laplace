// SPDX-License-Identifier: Apache-2.0
//! AxiomStepBuilder: translates `DecodedProbeEvent` → DPOR-consumable `AxiomEvent`.
//!
//! This is the critical bridge between the probe observation layer and the Axiom
//! DPOR verification engine. It solves two fundamental impedance mismatches:
//!
//! 1. **Thread ID space**: Kernel TIDs are arbitrary `u32` values. DPOR uses compact
//!    indices `0..MAX_AXIOM_THREADS` (= 8) backed by a single `u64` bitset.
//!    [`ThreadRegistry`] provides the monotonic mapping.
//!
//! 2. **Resource ID space**: Kernel resource IDs are arbitrary `u64` values (mutex
//!    virtual addresses, file descriptors). DPOR uses dense indices for its WFG
//!    adjacency matrix. [`ResourceRegistry`] provides the mapping.
//!
//! # Integration
//!
//! The output `AxiomStep` uses `usize` indices that map directly onto
//! `laplace_interfaces::ThreadId::new(step.thread)` and
//! `laplace_interfaces::ResourceId::new(step.resource)` at the integration boundary.
//! This keeps `laplace-probe-common` free of the heavy `laplace-interfaces` dependency.
//!
//! # Feature gating
//!
//! The registry types and builder are gated behind `#[cfg(feature = "std")]` because
//! they require `HashMap`. The core data types (`AxiomOp`, `AxiomStep`, `AxiomEvent`)
//! are unconditionally available and `no_std`-compatible.

#[cfg(feature = "std")]
use std::collections::HashMap;

#[cfg(feature = "std")]
use crate::decoder::DecodedProbeEvent;

#[cfg(feature = "scribe_docs")]
use laplace_macro::laplace_meta;

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Maximum concurrent threads — mirrors `laplace_axiom::dpor::MAX_THREADS = 8`.
///
/// This bound is enforced by the DPOR TinyBitSet (single u64). Kernel events
/// involving a TID that would exceed this limit are silently dropped and counted
/// in [`AxiomStepBuilder::overflow_count`].
#[cfg_attr(
    feature = "scribe_docs",
    laplace_meta(
        layer = "40_Probe_Common",
        link = "LEP-0013-laplace-probe-common_compaction_and_sovereignty"
    )
)]
pub const MAX_AXIOM_THREADS: usize = 8;

// ─────────────────────────────────────────────────────────────────────────────
// Core types (no_std-compatible)
// ─────────────────────────────────────────────────────────────────────────────

/// Dense DPOR thread index in `0..MAX_AXIOM_THREADS`.
///
/// Convert to `laplace_interfaces::ThreadId` at the integration boundary:
/// ```ignore
/// ThreadId::new(axiom_step.thread)
/// ```
pub type AxiomThreadId = usize;

/// Dense DPOR resource index.
///
/// Convert to `laplace_interfaces::ResourceId` at the integration boundary:
/// ```ignore
/// ResourceId::new(axiom_step.resource)
/// ```
pub type AxiomResourceId = usize;

/// Operation on a resource — mirrors `laplace_axiom::dpor::Operation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxiomOp {
    /// 배타적 락 획득 (Mutex::lock, RwLock::write)
    Request,
    /// 락 해제 (MutexGuard::drop, RwLockWriteGuard::drop)
    Release,
    /// 공유 락 획득 (RwLock::read)
    SharedRequest,
    /// 공유 락 해제 (RwLockReadGuard::drop)
    SharedRelease,
    /// 공유 읽기 (atomic load, channel recv)
    Read,
    /// 배타 쓰기 (atomic store, channel send)
    Write,
    /// 읽기-수정-쓰기 (CAS, fetch_add, fetch_sub)
    ReadWrite,
}

/// A single DPOR-consumable step, ready for conversion to `StepRecord`.
///
/// The `depth` and `clock` fields of `StepRecord` are managed by the DPOR
/// scheduler and must not be set here.
///
/// # Conversion to StepRecord
///
/// ```ignore
/// StepRecord {
///     thread: ThreadId::new(step.thread),
///     operation: match step.op {
///         AxiomOp::Request => Operation::Request,
///         AxiomOp::Release => Operation::Release,
///     },
///     resource: ResourceId::new(step.resource),
///     depth: 0,               // filled by DporScheduler
///     clock: VectorClock::new(), // filled by DporScheduler
/// }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct AxiomStep {
    /// Dense DPOR thread index (use `ThreadId::new(self.thread)` to convert).
    pub thread: AxiomThreadId,
    /// Operation type.
    pub op: AxiomOp,
    /// Dense DPOR resource index (use `ResourceId::new(self.resource)` to convert).
    pub resource: AxiomResourceId,
    /// Original kernel timestamp — useful for ordering and forensic reports.
    pub timestamp_ns: u64,
}

/// DPOR-consumable event produced by [`AxiomStepBuilder`].
///
/// `Step` variants feed directly into the DPOR scheduler. The lifecycle variants
/// (`ThreadSpawned`, `ThreadExited`, `SchedSwitch`) carry metadata that the
/// integration layer uses to update DPOR thread-status bookkeeping.
#[derive(Debug, Clone)]
pub enum AxiomEvent {
    /// A concrete resource operation — pass to the DPOR scheduler as a `StepRecord`.
    Step(AxiomStep),
    /// A new thread was spawned; register `child` with the DPOR scheduler.
    ThreadSpawned {
        parent: AxiomThreadId,
        child: AxiomThreadId,
        timestamp_ns: u64,
    },
    /// A thread exited; mark it `Completed` in the DPOR scheduler.
    ThreadExited {
        thread: AxiomThreadId,
        timestamp_ns: u64,
    },
    /// The CPU scheduler switched threads; update `Running`/`Blocked` status.
    SchedSwitch {
        prev: AxiomThreadId,
        next: AxiomThreadId,
        timestamp_ns: u64,
    },
}

// ─────────────────────────────────────────────────────────────────────────────
// ThreadRegistry (std only)
// ─────────────────────────────────────────────────────────────────────────────

/// Maps kernel thread IDs (`u32`) to compact DPOR indices (`0..MAX_AXIOM_THREADS`).
///
/// Allocates indices in first-seen order. Once `MAX_AXIOM_THREADS` slots are
/// exhausted, [`get_or_insert`] returns `None` and the caller should increment
/// its overflow counter.
///
/// Supports O(1) reverse lookup via a secondary `Vec` so that forensic reports
/// can translate internal `AxiomThreadId` indices back to the original OS TID.
///
/// [`get_or_insert`]: ThreadRegistry::get_or_insert
#[cfg(feature = "std")]
#[derive(Debug)]
pub struct ThreadRegistry {
    map: HashMap<u32, AxiomThreadId>,
    /// Reverse index: `reverse[axiom_id]` = original kernel TID.
    reverse: Vec<u32>,
    next_id: usize,
}

#[cfg(feature = "std")]
impl ThreadRegistry {
    /// Creates a new empty registry.
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            reverse: Vec::new(),
            next_id: 0,
        }
    }

    /// Returns the DPOR index for `kernel_tid`, allocating one on first encounter.
    ///
    /// Returns `None` if `MAX_AXIOM_THREADS` has been reached.
    pub fn get_or_insert(&mut self, kernel_tid: u32) -> Option<AxiomThreadId> {
        if let Some(&id) = self.map.get(&kernel_tid) {
            return Some(id);
        }
        if self.next_id >= MAX_AXIOM_THREADS {
            return None;
        }
        let id = self.next_id;
        self.map.insert(kernel_tid, id);
        self.reverse.push(kernel_tid);
        self.next_id += 1;
        Some(id)
    }

    /// Looks up the DPOR index for `kernel_tid` without allocating a new slot.
    ///
    /// Returns `None` if the TID has never been seen.
    pub fn get(&self, kernel_tid: u32) -> Option<AxiomThreadId> {
        self.map.get(&kernel_tid).copied()
    }

    /// **Reverse lookup** — returns the original kernel TID for a DPOR index.
    ///
    /// This is the key forensic primitive: when the Oracle reports a violation
    /// using `ThreadId(0)`, call `get_kernel_tid(0)` to recover the OS TID.
    ///
    /// O(1) — backed by a secondary `Vec`.
    pub fn get_kernel_tid(&self, axiom_id: AxiomThreadId) -> Option<u32> {
        self.reverse.get(axiom_id).copied()
    }

    /// Number of kernel TIDs currently registered.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Returns `true` if no threads are registered.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(feature = "std")]
impl Default for ThreadRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ResourceRegistry (std only)
// ─────────────────────────────────────────────────────────────────────────────

/// Maps kernel resource IDs (`u64` mutex addresses / file descriptors) to compact
/// DPOR resource indices.
///
/// Allocates indices in first-seen order. No upper bound is enforced here; the
/// DPOR WFG matrix is dynamically sized by the scheduler.
///
/// Supports O(1) reverse lookup via a secondary `Vec` so that forensic reports
/// can translate internal `ResourceId` indices back to the original kernel address.
#[cfg(feature = "std")]
#[derive(Debug)]
pub struct ResourceRegistry {
    map: HashMap<u64, AxiomResourceId>,
    /// Reverse index: `reverse[axiom_id]` = original kernel resource ID.
    reverse: Vec<u64>,
    next_id: usize,
}

#[cfg(feature = "std")]
impl ResourceRegistry {
    /// Creates a new empty registry.
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            reverse: Vec::new(),
            next_id: 0,
        }
    }

    /// Returns the DPOR index for `kernel_resource_id`, allocating one on first encounter.
    pub fn get_or_insert(&mut self, kernel_resource_id: u64) -> AxiomResourceId {
        if let Some(&id) = self.map.get(&kernel_resource_id) {
            return id;
        }
        let id = self.next_id;
        self.map.insert(kernel_resource_id, id);
        self.reverse.push(kernel_resource_id);
        self.next_id += 1;
        id
    }

    /// Looks up the DPOR index for `kernel_resource_id` without allocating.
    ///
    /// Returns `None` if the resource has never been seen.
    pub fn get(&self, kernel_resource_id: u64) -> Option<AxiomResourceId> {
        self.map.get(&kernel_resource_id).copied()
    }

    /// **Reverse lookup** — returns the original kernel resource ID for a DPOR index.
    ///
    /// This is the key forensic primitive: when the Oracle reports a violation
    /// on `ResourceId(1)`, call `get_kernel_resource_id(1)` to recover the original
    /// mutex virtual address or file descriptor number.
    ///
    /// O(1) — backed by a secondary `Vec`.
    pub fn get_kernel_resource_id(&self, axiom_id: AxiomResourceId) -> Option<u64> {
        self.reverse.get(axiom_id).copied()
    }

    /// Number of kernel resource IDs currently registered.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Returns `true` if no resources are registered.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(feature = "std")]
impl Default for ResourceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AxiomStepBuilder (std only)
// ─────────────────────────────────────────────────────────────────────────────

/// Translates a `DecodedProbeEvent` stream into DPOR-consumable `AxiomEvent`s.
///
/// Maintains [`ThreadRegistry`] and [`ResourceRegistry`] across the lifetime of
/// a verification session so that DPOR index assignments remain stable.
///
/// # Mapping table
///
/// | `DecodedProbeEvent`        | `AxiomOp`  | Resource key      | Notes                          |
/// |----------------------------|------------|-------------------|--------------------------------|
/// | `NetworkRequest`           | `Request`  | `resource_id` (fd)| Inbound L7 = acquire resource  |
/// | `NetworkResponse`          | `Release`  | `resource_id` (fd)| Outbound L7 = release resource |
/// | `LockAcquire`              | `Request`  | `mutex_addr`      | Thread wants lock              |
/// | `LockAcquired`             | `Request`  | `mutex_addr`      | Lock obtained (after wait)     |
/// | `LockRelease`              | `Release`  | `mutex_addr`      | Thread frees lock              |
/// | `LockContention`           | `Request`  | `mutex_addr`      | Blocked waiting for lock       |
/// | `ConnOpen`                 | `Request`  | `resource_id` (fd)| Connection fd acquired         |
/// | `ConnClose`                | `Release`  | `resource_id` (fd)| Connection fd released         |
/// | `ThreadSpawn`              | —          | —                 | `AxiomEvent::ThreadSpawned`    |
/// | `ThreadExit`               | —          | —                 | `AxiomEvent::ThreadExited`     |
/// | `SchedSwitch`              | —          | —                 | `AxiomEvent::SchedSwitch`      |
///
/// # Example
///
/// ```ignore
/// let decoder = ProbeEventDecoder::new();
/// let mut builder = AxiomStepBuilder::new();
///
/// for raw in raw_events {
///     if let Some(decoded) = decoder.decode(&raw) {
///         if let Some(event) = builder.process(&decoded) {
///             match event {
///                 AxiomEvent::Step(step) => {
///                     scheduler.record(StepRecord {
///                         thread: ThreadId::new(step.thread),
///                         operation: step.op.into(),
///                         resource: ResourceId::new(step.resource),
///                         depth: 0,
///                         clock: VectorClock::new(),
///                     });
///                 }
///                 AxiomEvent::ThreadSpawned { child, .. } => { /* register child */ }
///                 AxiomEvent::ThreadExited { thread, .. } => { /* mark Completed */ }
///                 AxiomEvent::SchedSwitch { prev, next, .. } => { /* update status */ }
///             }
///         }
///     }
/// }
/// ```
#[cfg(feature = "std")]
#[derive(Debug)]
pub struct AxiomStepBuilder {
    threads: ThreadRegistry,
    resources: ResourceRegistry,
    /// Count of events dropped because all `MAX_AXIOM_THREADS` slots were full.
    overflow_count: u64,
}

#[cfg(feature = "std")]
impl AxiomStepBuilder {
    /// Creates a new builder with empty registries.
    pub fn new() -> Self {
        Self {
            threads: ThreadRegistry::new(),
            resources: ResourceRegistry::new(),
            overflow_count: 0,
        }
    }

    /// Translates a single `DecodedProbeEvent` into an `AxiomEvent`.
    ///
    /// Returns `None` when:
    /// - The event involves a thread ID that cannot be allocated (thread cap reached).
    /// - The event is a `ThreadExit` or `SchedSwitch` for a TID that was never seen
    ///   (the thread pre-dates the observation window).
    #[cfg_attr(
        feature = "scribe_docs",
        laplace_meta(
            layer = "40_Probe_Common",
            link = "LEP-0013-laplace-probe-common_compaction_and_sovereignty"
        )
    )]
    pub fn process(&mut self, event: &DecodedProbeEvent) -> Option<AxiomEvent> {
        match event {
            DecodedProbeEvent::NetworkRequest {
                tid,
                timestamp_ns,
                resource_id,
                ..
            } => {
                let thread = self.resolve_thread(*tid)?;
                let resource = self.resources.get_or_insert(*resource_id);
                Some(AxiomEvent::Step(AxiomStep {
                    thread,
                    op: AxiomOp::Request,
                    resource,
                    timestamp_ns: *timestamp_ns,
                }))
            }

            DecodedProbeEvent::NetworkResponse {
                tid,
                timestamp_ns,
                resource_id,
                ..
            } => {
                let thread = self.resolve_thread(*tid)?;
                let resource = self.resources.get_or_insert(*resource_id);
                Some(AxiomEvent::Step(AxiomStep {
                    thread,
                    op: AxiomOp::Release,
                    resource,
                    timestamp_ns: *timestamp_ns,
                }))
            }

            // LockAcquire = thread calls pthread_mutex_lock() — emit ONE Request per
            // acquisition attempt.  The DPOR engine internally determines whether the
            // request is granted (Running) or blocked (Blocked) based on resource state.
            DecodedProbeEvent::LockAcquire {
                tid,
                timestamp_ns,
                mutex_addr,
                ..
            } => {
                let thread = self.resolve_thread(*tid)?;
                let resource = self.resources.get_or_insert(*mutex_addr);
                Some(AxiomEvent::Step(AxiomStep {
                    thread,
                    op: AxiomOp::Request,
                    resource,
                    timestamp_ns: *timestamp_ns,
                }))
            }

            // LockAcquired = kernel confirmed the lock is now held.  This is a secondary
            // event that pairs with LockAcquire; emitting a second Request here would
            // create a spurious self-deadlock in the DPOR model.  Drop it.
            DecodedProbeEvent::LockAcquired { .. } => None,

            DecodedProbeEvent::LockRelease {
                tid,
                timestamp_ns,
                mutex_addr,
            } => {
                let thread = self.resolve_thread(*tid)?;
                let resource = self.resources.get_or_insert(*mutex_addr);
                Some(AxiomEvent::Step(AxiomStep {
                    thread,
                    op: AxiomOp::Release,
                    resource,
                    timestamp_ns: *timestamp_ns,
                }))
            }

            // LockContention = kernel put the thread to sleep because the lock is contended.
            // This pairs with the preceding LockAcquire for the same mutex.  Emitting a
            // second Request here would double-count the acquisition attempt.  Drop it.
            DecodedProbeEvent::LockContention { .. } => None,

            DecodedProbeEvent::ThreadSpawn {
                parent_tid,
                child_tid,
                timestamp_ns,
            } => {
                let parent = self.resolve_thread(*parent_tid)?;
                let child = self.resolve_thread(*child_tid)?;
                Some(AxiomEvent::ThreadSpawned {
                    parent,
                    child,
                    timestamp_ns: *timestamp_ns,
                })
            }

            DecodedProbeEvent::ThreadExit { tid, timestamp_ns } => {
                // Use get (not get_or_insert) — a thread that exits without
                // ever appearing before is outside our observation window.
                let thread = self.threads.get(*tid)?;
                Some(AxiomEvent::ThreadExited {
                    thread,
                    timestamp_ns: *timestamp_ns,
                })
            }

            DecodedProbeEvent::SchedSwitch {
                prev_tid,
                next_tid,
                timestamp_ns,
                ..
            } => {
                // prev must already be known; next may be newly observed.
                let prev = self.threads.get(*prev_tid)?;
                let next = self.resolve_thread(*next_tid)?;
                Some(AxiomEvent::SchedSwitch {
                    prev,
                    next,
                    timestamp_ns: *timestamp_ns,
                })
            }

            DecodedProbeEvent::ConnOpen {
                tid,
                timestamp_ns,
                resource_id,
                ..
            } => {
                let thread = self.resolve_thread(*tid)?;
                let resource = self.resources.get_or_insert(*resource_id);
                Some(AxiomEvent::Step(AxiomStep {
                    thread,
                    op: AxiomOp::Request,
                    resource,
                    timestamp_ns: *timestamp_ns,
                }))
            }

            DecodedProbeEvent::ConnClose {
                tid,
                timestamp_ns,
                resource_id,
            } => {
                let thread = self.resolve_thread(*tid)?;
                let resource = self.resources.get_or_insert(*resource_id);
                Some(AxiomEvent::Step(AxiomStep {
                    thread,
                    op: AxiomOp::Release,
                    resource,
                    timestamp_ns: *timestamp_ns,
                }))
            }

            DecodedProbeEvent::RwLockReadAcquire {
                tid,
                timestamp_ns,
                rwlock_addr,
            } => {
                let thread = self.resolve_thread(*tid)?;
                let resource = self.resources.get_or_insert(*rwlock_addr);
                Some(AxiomEvent::Step(AxiomStep {
                    thread,
                    op: AxiomOp::SharedRequest,
                    resource,
                    timestamp_ns: *timestamp_ns,
                }))
            }

            DecodedProbeEvent::RwLockReadRelease {
                tid,
                timestamp_ns,
                rwlock_addr,
            } => {
                let thread = self.resolve_thread(*tid)?;
                let resource = self.resources.get_or_insert(*rwlock_addr);
                Some(AxiomEvent::Step(AxiomStep {
                    thread,
                    op: AxiomOp::SharedRelease,
                    resource,
                    timestamp_ns: *timestamp_ns,
                }))
            }

            DecodedProbeEvent::RwLockWriteAcquire {
                tid,
                timestamp_ns,
                rwlock_addr,
            } => {
                let thread = self.resolve_thread(*tid)?;
                let resource = self.resources.get_or_insert(*rwlock_addr);
                Some(AxiomEvent::Step(AxiomStep {
                    thread,
                    op: AxiomOp::Request,
                    resource,
                    timestamp_ns: *timestamp_ns,
                }))
            }

            DecodedProbeEvent::RwLockWriteRelease {
                tid,
                timestamp_ns,
                rwlock_addr,
            } => {
                let thread = self.resolve_thread(*tid)?;
                let resource = self.resources.get_or_insert(*rwlock_addr);
                Some(AxiomEvent::Step(AxiomStep {
                    thread,
                    op: AxiomOp::Release,
                    resource,
                    timestamp_ns: *timestamp_ns,
                }))
            }

            DecodedProbeEvent::AtomicLoad {
                tid,
                timestamp_ns,
                addr,
            } => {
                let thread = self.resolve_thread(*tid)?;
                let resource = self.resources.get_or_insert(*addr);
                Some(AxiomEvent::Step(AxiomStep {
                    thread,
                    op: AxiomOp::Read,
                    resource,
                    timestamp_ns: *timestamp_ns,
                }))
            }

            DecodedProbeEvent::AtomicStore {
                tid,
                timestamp_ns,
                addr,
            } => {
                let thread = self.resolve_thread(*tid)?;
                let resource = self.resources.get_or_insert(*addr);
                Some(AxiomEvent::Step(AxiomStep {
                    thread,
                    op: AxiomOp::Write,
                    resource,
                    timestamp_ns: *timestamp_ns,
                }))
            }

            DecodedProbeEvent::AtomicRmw {
                tid,
                timestamp_ns,
                addr,
            } => {
                let thread = self.resolve_thread(*tid)?;
                let resource = self.resources.get_or_insert(*addr);
                Some(AxiomEvent::Step(AxiomStep {
                    thread,
                    op: AxiomOp::ReadWrite,
                    resource,
                    timestamp_ns: *timestamp_ns,
                }))
            }

            DecodedProbeEvent::SemaphoreAcquire {
                tid,
                timestamp_ns,
                sem_addr,
            } => {
                let thread = self.resolve_thread(*tid)?;
                let resource = self.resources.get_or_insert(*sem_addr);
                Some(AxiomEvent::Step(AxiomStep {
                    thread,
                    op: AxiomOp::Request,
                    resource,
                    timestamp_ns: *timestamp_ns,
                }))
            }

            DecodedProbeEvent::SemaphoreRelease {
                tid,
                timestamp_ns,
                sem_addr,
            } => {
                let thread = self.resolve_thread(*tid)?;
                let resource = self.resources.get_or_insert(*sem_addr);
                Some(AxiomEvent::Step(AxiomStep {
                    thread,
                    op: AxiomOp::Release,
                    resource,
                    timestamp_ns: *timestamp_ns,
                }))
            }

            DecodedProbeEvent::ChannelSend {
                tid,
                timestamp_ns,
                channel_addr,
            } => {
                let thread = self.resolve_thread(*tid)?;
                let resource = self.resources.get_or_insert(*channel_addr);
                Some(AxiomEvent::Step(AxiomStep {
                    thread,
                    op: AxiomOp::Write,
                    resource,
                    timestamp_ns: *timestamp_ns,
                }))
            }

            DecodedProbeEvent::ChannelRecv {
                tid,
                timestamp_ns,
                channel_addr,
            } => {
                let thread = self.resolve_thread(*tid)?;
                let resource = self.resources.get_or_insert(*channel_addr);
                Some(AxiomEvent::Step(AxiomStep {
                    thread,
                    op: AxiomOp::Read,
                    resource,
                    timestamp_ns: *timestamp_ns,
                }))
            }
        }
    }

    /// Translates a batch of decoded events, silently dropping those that return `None`.
    pub fn process_batch(&mut self, events: &[DecodedProbeEvent]) -> Vec<AxiomEvent> {
        events.iter().filter_map(|e| self.process(e)).collect()
    }

    /// Read-only view of the thread registry (for diagnostics / integration).
    pub fn thread_registry(&self) -> &ThreadRegistry {
        &self.threads
    }

    /// Read-only view of the resource registry (for diagnostics / integration).
    pub fn resource_registry(&self) -> &ResourceRegistry {
        &self.resources
    }

    /// Number of events dropped because the `MAX_AXIOM_THREADS` cap was hit.
    pub fn overflow_count(&self) -> u64 {
        self.overflow_count
    }

    /// Resolves a kernel TID to a DPOR thread index, allocating if new.
    ///
    /// Increments the overflow counter when the thread cap is reached.
    fn resolve_thread(&mut self, kernel_tid: u32) -> Option<AxiomThreadId> {
        match self.threads.get_or_insert(kernel_tid) {
            Some(id) => Some(id),
            None => {
                self.overflow_count += 1;
                None
            }
        }
    }
}

#[cfg(feature = "std")]
impl Default for AxiomStepBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::decoder::DecodedProbeEvent;

    // ── ThreadRegistry ──────────────────────────────────────────────────────

    #[test]
    fn thread_registry_allocates_monotonic_ids() {
        let mut reg = ThreadRegistry::new();
        assert_eq!(reg.get_or_insert(100), Some(0));
        assert_eq!(reg.get_or_insert(200), Some(1));
        assert_eq!(reg.get_or_insert(100), Some(0)); // idempotent
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn thread_registry_returns_none_at_cap() {
        let mut reg = ThreadRegistry::new();
        for i in 0..MAX_AXIOM_THREADS {
            assert!(reg.get_or_insert(i as u32).is_some());
        }
        assert_eq!(reg.get_or_insert(999), None);
    }

    #[test]
    fn thread_registry_get_does_not_allocate() {
        let mut reg = ThreadRegistry::new();
        assert!(reg.get(42).is_none());
        reg.get_or_insert(42);
        assert_eq!(reg.get(42), Some(0));
        assert_eq!(reg.len(), 1);
    }

    // ── ResourceRegistry ────────────────────────────────────────────────────

    #[test]
    fn resource_registry_allocates_monotonic_ids() {
        let mut reg = ResourceRegistry::new();
        assert_eq!(reg.get_or_insert(0xAAAA_0000), 0);
        assert_eq!(reg.get_or_insert(0xBBBB_0000), 1);
        assert_eq!(reg.get_or_insert(0xAAAA_0000), 0); // idempotent
    }

    #[test]
    fn thread_registry_reverse_lookup() {
        let mut reg = ThreadRegistry::new();
        reg.get_or_insert(1001);
        reg.get_or_insert(1002);
        assert_eq!(reg.get_kernel_tid(0), Some(1001));
        assert_eq!(reg.get_kernel_tid(1), Some(1002));
        assert_eq!(reg.get_kernel_tid(2), None); // out of range
    }

    #[test]
    fn resource_registry_reverse_lookup() {
        let mut reg = ResourceRegistry::new();
        reg.get_or_insert(0xDEAD_0000);
        reg.get_or_insert(0xBEEF_0000);
        assert_eq!(reg.get_kernel_resource_id(0), Some(0xDEAD_0000));
        assert_eq!(reg.get_kernel_resource_id(1), Some(0xBEEF_0000));
        assert_eq!(reg.get_kernel_resource_id(2), None);
    }

    // ── AxiomStepBuilder ────────────────────────────────────────────────────

    #[test]
    fn builder_lock_acquire_produces_request_step() {
        let mut builder = AxiomStepBuilder::new();
        let event = DecodedProbeEvent::LockAcquire {
            tid: 10,
            timestamp_ns: 1_000,
            mutex_addr: 0xDEAD_0000,
            contention_ns: 0,
        };
        match builder.process(&event).unwrap() {
            AxiomEvent::Step(step) => {
                assert_eq!(step.op, AxiomOp::Request);
                assert_eq!(step.thread, 0);
                assert_eq!(step.resource, 0);
                assert_eq!(step.timestamp_ns, 1_000);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn builder_lock_release_produces_release_step() {
        let mut builder = AxiomStepBuilder::new();
        // Must see LockAcquire first so the thread gets a slot.
        builder.process(&DecodedProbeEvent::LockAcquire {
            tid: 10,
            timestamp_ns: 100,
            mutex_addr: 0xDEAD_0000,
            contention_ns: 0,
        });
        let event = DecodedProbeEvent::LockRelease {
            tid: 10,
            timestamp_ns: 200,
            mutex_addr: 0xDEAD_0000,
        };
        match builder.process(&event).unwrap() {
            AxiomEvent::Step(step) => {
                assert_eq!(step.op, AxiomOp::Release);
                assert_eq!(step.thread, 0); // same tid → same index
                assert_eq!(step.resource, 0); // same addr → same index
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn builder_thread_exit_returns_none_for_unseen_tid() {
        let mut builder = AxiomStepBuilder::new();
        let event = DecodedProbeEvent::ThreadExit {
            tid: 99,
            timestamp_ns: 0,
        };
        // tid 99 was never seen → None
        assert!(builder.process(&event).is_none());
    }

    #[test]
    fn builder_thread_spawn_produces_spawned_event() {
        let mut builder = AxiomStepBuilder::new();
        let event = DecodedProbeEvent::ThreadSpawn {
            parent_tid: 1,
            child_tid: 2,
            timestamp_ns: 500,
        };
        match builder.process(&event).unwrap() {
            AxiomEvent::ThreadSpawned {
                parent,
                child,
                timestamp_ns,
            } => {
                assert_eq!(parent, 0);
                assert_eq!(child, 1);
                assert_eq!(timestamp_ns, 500);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn builder_different_mutexes_get_distinct_resource_ids() {
        let mut builder = AxiomStepBuilder::new();
        let e1 = DecodedProbeEvent::LockAcquire {
            tid: 1,
            timestamp_ns: 0,
            mutex_addr: 0x1000,
            contention_ns: 0,
        };
        let e2 = DecodedProbeEvent::LockAcquire {
            tid: 2,
            timestamp_ns: 0,
            mutex_addr: 0x2000,
            contention_ns: 0,
        };
        let step1 = match builder.process(&e1).unwrap() {
            AxiomEvent::Step(s) => s,
            _ => panic!(),
        };
        let step2 = match builder.process(&e2).unwrap() {
            AxiomEvent::Step(s) => s,
            _ => panic!(),
        };
        // Different mutex addresses must produce different resource indices
        assert_ne!(step1.resource, step2.resource);
    }

    #[test]
    fn builder_overflow_counter_increments_at_thread_cap() {
        let mut builder = AxiomStepBuilder::new();
        // Fill all 8 thread slots
        for i in 0..MAX_AXIOM_THREADS {
            let e = DecodedProbeEvent::ThreadExit {
                tid: i as u32,
                timestamp_ns: 0,
            };
            // ThreadExit uses `get` (not `get_or_insert`), so these don't count.
            // Use LockAcquire instead to force allocation.
            builder.process(&DecodedProbeEvent::LockAcquire {
                tid: i as u32,
                timestamp_ns: 0,
                mutex_addr: 0,
                contention_ns: 0,
            });
            let _ = e;
        }
        assert_eq!(builder.overflow_count(), 0);

        // Next new TID should overflow
        builder.process(&DecodedProbeEvent::LockAcquire {
            tid: 999,
            timestamp_ns: 0,
            mutex_addr: 0,
            contention_ns: 0,
        });
        assert_eq!(builder.overflow_count(), 1);
    }

    #[test]
    fn builder_ab_ba_deadlock_scenario_assigns_two_distinct_resources() {
        // Thread A: acquire lock_x, then request lock_y
        // Thread B: acquire lock_y, then request lock_x
        //
        // LockContention events are dropped by the adapter (they pair with LockAcquire
        // and would create duplicate Requests in the DPOR model).
        let lock_x: u64 = 0xAAAA_0000;
        let lock_y: u64 = 0xBBBB_0000;
        let tid_a: u32 = 1;
        let tid_b: u32 = 2;

        let mut builder = AxiomStepBuilder::new();
        let events = [
            DecodedProbeEvent::LockAcquire {
                tid: tid_a,
                timestamp_ns: 1,
                mutex_addr: lock_x,
                contention_ns: 0,
            },
            DecodedProbeEvent::LockAcquire {
                tid: tid_b,
                timestamp_ns: 2,
                mutex_addr: lock_y,
                contention_ns: 0,
            },
            // LockContention is now dropped — only 2 Steps emitted
            DecodedProbeEvent::LockContention {
                tid: tid_a,
                timestamp_ns: 3,
                mutex_addr: lock_y,
            },
            DecodedProbeEvent::LockContention {
                tid: tid_b,
                timestamp_ns: 4,
                mutex_addr: lock_x,
            },
        ];

        let axiom_events = builder.process_batch(&events);
        // Only the two LockAcquire events produce Steps; LockContention is dropped
        assert_eq!(axiom_events.len(), 2);

        // Threads A and B must have distinct indices
        let ta = builder.thread_registry().get(tid_a).unwrap();
        let tb = builder.thread_registry().get(tid_b).unwrap();
        assert_ne!(ta, tb);

        // lock_x and lock_y must have distinct resource indices
        let rx = builder.resource_registry().get(lock_x).unwrap();
        let ry = builder.resource_registry().get(lock_y).unwrap();
        assert_ne!(rx, ry);
    }
}
