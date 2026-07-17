// SPDX-License-Identifier: Apache-2.0
//! Evidence-only wrap-real surface for `arc_swap` 1.9.
//!
//! ## Provided surface
//!
//! [`ModelArcSwap`] provides `new`, `load`, `load_full`, and `store` for the
//! ordinary `ArcSwap` use case. [`ModelArcSwapOption`] provides `new`,
//! `empty`, `load`, `load_full`, and `store` for `ArcSwapOption`. [`Cache`]
//! provides construction and `load`, with fresh-load-equivalent version
//! evidence.
//!
//! ## Loud residual cuts
//!
//! `swap`, `rcu`, `compare_and_swap`, `ArcSwapAny`, `ArcSwapWeak`, custom
//! strategies, `access::Map`, and every other `arc_swap` API are intentionally
//! absent. Using one of those surfaces against this module fails at compile
//! time instead of silently receiving an approximation. Static automatic
//! instrumentation is also outside this opt-in surface.
//!
//! ## Version honesty
//!
//! Every stored value is paired with its version inside the same real
//! `ArcSwap` snapshot. A load reports the version read from that snapshot;
//! it never reads a side counter after loading. Stores are serialized only for
//! version allocation/publication, so concurrent stores receive one linear
//! sequence while loads remain wait-free on the underlying `ArcSwap`.

use std::ops::Deref;
use std::sync::{Arc, Mutex};

use crate::hooks::{async_cell_hook, next_async_lock_resource_id};

struct Versioned<T> {
    version: u64,
    value: Arc<T>,
}

/// `ArcSwap`-compatible evidence-only model cell.
pub struct ModelArcSwap<T> {
    inner: ::arc_swap::ArcSwap<Versioned<T>>,
    resource: u64,
    next_version: Mutex<u64>,
}

impl<T> ModelArcSwap<T> {
    /// Creates a model cell holding `value` at version zero.
    #[must_use]
    pub fn new(value: T) -> Self {
        let resource = next_async_lock_resource_id();
        let cell = Self {
            inner: ::arc_swap::ArcSwap::new(Arc::new(Versioned {
                version: 0,
                value: Arc::new(value),
            })),
            resource,
            next_version: Mutex::new(0),
        };
        if let Some(hook) = async_cell_hook() {
            hook.cell_created(resource);
        }
        cell
    }

    /// Loads a temporary guard over the value in one observed snapshot.
    pub fn load(&self) -> ModelArcSwapGuard<T> {
        let inner = self.inner.load();
        let version = inner.version;
        let value = Arc::clone(&inner.value);
        if let Some(hook) = async_cell_hook() {
            hook.cell_load(self.resource, version);
        }
        ModelArcSwapGuard {
            _inner: inner,
            value,
        }
    }

    /// Clones the value from one observed snapshot.
    #[must_use]
    pub fn load_full(&self) -> Arc<T> {
        let snapshot = self.inner.load_full();
        self.report_load(snapshot.version);
        Arc::clone(&snapshot.value)
    }

    /// Publishes `value` as the next version.
    ///
    /// # Panics
    ///
    /// Panics if the process-local version mutex is poisoned or the version
    /// counter is exhausted.
    pub fn store(&self, value: Arc<T>) {
        let mut next_version = self
            .next_version
            .lock()
            .expect("cell version lock poisoned");
        *next_version = next_version
            .checked_add(1)
            .expect("cell version counter exhausted");
        let version = *next_version;
        self.inner.store(Arc::new(Versioned { version, value }));
        if let Some(hook) = async_cell_hook() {
            hook.cell_store(self.resource, version);
        }
    }

    fn report_load(&self, version: u64) {
        if let Some(hook) = async_cell_hook() {
            hook.cell_load(self.resource, version);
        }
    }

    fn load_snapshot(&self) -> (Arc<T>, u64) {
        let snapshot = self.inner.load_full();
        let version = snapshot.version;
        let value = Arc::clone(&snapshot.value);
        self.report_load(version);
        (value, version)
    }

    fn initial_snapshot(&self) -> (Arc<T>, u64) {
        let snapshot = self.inner.load_full();
        (Arc::clone(&snapshot.value), snapshot.version)
    }
}

/// Temporary guard returned by [`ModelArcSwap::load`].
pub struct ModelArcSwapGuard<T> {
    _inner: ::arc_swap::Guard<Arc<Versioned<T>>>,
    value: Arc<T>,
}

impl<T> Deref for ModelArcSwapGuard<T> {
    type Target = Arc<T>;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

/// `ArcSwapOption`-compatible evidence-only model cell.
pub struct ModelArcSwapOption<T> {
    inner: ::arc_swap::ArcSwapOption<Versioned<T>>,
    resource: u64,
    next_version: Mutex<u64>,
}

impl<T> ModelArcSwapOption<T> {
    /// Creates a model option cell at version zero.
    #[must_use]
    pub fn new(value: Option<Arc<T>>) -> Self {
        let resource = next_async_lock_resource_id();
        let cell = Self {
            inner: ::arc_swap::ArcSwapOption::new(
                value.map(|value| Arc::new(Versioned { version: 0, value })),
            ),
            resource,
            next_version: Mutex::new(0),
        };
        if let Some(hook) = async_cell_hook() {
            hook.cell_created(resource);
        }
        cell
    }

    /// Creates an empty model option cell at version zero.
    #[must_use]
    pub fn empty() -> Self {
        Self::new(None)
    }

    /// Loads a temporary guard over one observed option snapshot.
    pub fn load(&self) -> ModelArcSwapOptionGuard<T> {
        let inner = self.inner.load();
        let value = inner.as_ref().map(|snapshot| Arc::clone(&snapshot.value));
        if let Some(snapshot) = inner.as_ref() {
            if let Some(hook) = async_cell_hook() {
                hook.cell_load(self.resource, snapshot.version);
            }
        }
        ModelArcSwapOptionGuard { inner: value }
    }

    /// Clones the optional value from one observed snapshot.
    #[must_use]
    pub fn load_full(&self) -> Option<Arc<T>> {
        let snapshot = self.inner.load_full();
        if let Some(snapshot) = snapshot.as_ref() {
            if let Some(hook) = async_cell_hook() {
                hook.cell_load(self.resource, snapshot.version);
            }
            Some(Arc::clone(&snapshot.value))
        } else {
            None
        }
    }

    /// Publishes the next optional cell version.
    ///
    /// # Panics
    ///
    /// Panics if the process-local version mutex is poisoned or the version
    /// counter is exhausted.
    pub fn store(&self, value: Option<Arc<T>>) {
        let mut next_version = self
            .next_version
            .lock()
            .expect("cell version lock poisoned");
        *next_version = next_version
            .checked_add(1)
            .expect("cell version counter exhausted");
        let version = *next_version;
        self.inner
            .store(value.map(|value| Arc::new(Versioned { version, value })));
        if let Some(hook) = async_cell_hook() {
            hook.cell_store(self.resource, version);
        }
    }
}

/// Snapshot guard returned by [`ModelArcSwapOption::load`].
pub struct ModelArcSwapOptionGuard<T> {
    inner: Option<Arc<T>>,
}

impl<T> Deref for ModelArcSwapOptionGuard<T> {
    type Target = Option<Arc<T>>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

/// Fresh-load-equivalent cache over a [`ModelArcSwap`].
pub struct Cache<'a, T> {
    source: &'a ModelArcSwap<T>,
    cached: Arc<T>,
    cached_version: u64,
}

impl<'a, T> Cache<'a, T> {
    /// Creates a cache from a model cell without adding an observation event.
    #[must_use]
    pub fn new(source: &'a ModelArcSwap<T>) -> Self {
        let (cached, cached_version) = source.initial_snapshot();
        Self {
            source,
            cached,
            cached_version,
        }
    }

    /// Revalidates against a fresh cell snapshot and returns the cached value.
    pub fn load(&mut self) -> &T {
        let (value, version) = self.source.load_snapshot();
        if version != self.cached_version {
            self.cached = value;
            self.cached_version = version;
        }
        &self.cached
    }
}
