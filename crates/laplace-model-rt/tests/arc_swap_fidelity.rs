// SPDX-License-Identifier: Apache-2.0
#![cfg(feature = "arc-swap")]

//! Native-vs-model fidelity and version-honesty tests for the A-1 surface.

use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError};

use laplace_model_rt::{
    clear_async_cell_hook, install_async_cell_hook, reset_model_async_ids_for_model, AsyncCellHook,
    Cache, ModelArcSwap, ModelArcSwapOption,
};

static TEST_GUARD: StdMutex<()> = StdMutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    TEST_GUARD.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CellEvent {
    Created(u64),
    Load(u64, u64),
    Store(u64, u64),
}

#[derive(Default)]
struct RecordingCellHook {
    events: StdMutex<Vec<CellEvent>>,
}

impl RecordingCellHook {
    fn take(&self) -> Vec<CellEvent> {
        std::mem::take(&mut *self.events.lock().expect("cell events lock"))
    }
}

impl AsyncCellHook for RecordingCellHook {
    fn cell_created(&self, resource: u64) {
        self.events
            .lock()
            .expect("cell events lock")
            .push(CellEvent::Created(resource));
    }

    fn cell_load(&self, resource: u64, version: u64) {
        self.events
            .lock()
            .expect("cell events lock")
            .push(CellEvent::Load(resource, version));
    }

    fn cell_store(&self, resource: u64, version: u64) {
        self.events
            .lock()
            .expect("cell events lock")
            .push(CellEvent::Store(resource, version));
    }
}

#[test]
fn model_arc_swap_matches_native_values_and_return_contract() {
    let _serial = serial();
    reset_model_async_ids_for_model();
    clear_async_cell_hook();

    let native = ::arc_swap::ArcSwap::from_pointee(1_u64);
    let model = ModelArcSwap::new(1_u64);
    assert_eq!(**native.load(), **model.load());
    assert_eq!(*native.load_full(), *model.load_full());

    native.store(Arc::new(2));
    model.store(Arc::new(2));
    assert_eq!(**native.load(), **model.load());
    assert_eq!(*native.load_full(), *model.load_full());
}

#[test]
fn model_arc_swap_option_matches_none_some_transitions() {
    let _serial = serial();
    reset_model_async_ids_for_model();
    clear_async_cell_hook();

    let native: ::arc_swap::ArcSwapOption<u64> = ::arc_swap::ArcSwapOption::empty();
    let model: ModelArcSwapOption<u64> = ModelArcSwapOption::empty();
    assert_eq!(native.load_full(), model.load_full());
    assert!(model.load().is_none());

    native.store(Some(Arc::new(9)));
    model.store(Some(Arc::new(9)));
    assert_eq!(native.load_full(), model.load_full());

    native.store(None);
    model.store(None);
    assert_eq!(native.load_full(), model.load_full());
}

#[test]
fn cache_load_revalidates_like_a_fresh_load_and_reports_the_same_version() {
    let _serial = serial();
    reset_model_async_ids_for_model();
    let hook = Arc::new(RecordingCellHook::default());
    install_async_cell_hook(hook.clone());

    let model = ModelArcSwap::new(10_u64);
    let mut cache = Cache::new(&model);
    model.store(Arc::new(11));
    assert_eq!(*cache.load(), 11);
    assert_eq!(*model.load_full(), 11);

    let events = hook.take();
    assert_eq!(events[0], CellEvent::Created(1));
    assert_eq!(events[1], CellEvent::Store(1, 1));
    assert_eq!(events[2], CellEvent::Load(1, 1));
    assert_eq!(events[3], CellEvent::Load(1, 1));
    clear_async_cell_hook();
}

#[test]
fn concurrent_stores_are_serialized_and_load_versions_are_snapshot_versions() {
    let _serial = serial();
    reset_model_async_ids_for_model();
    let hook = Arc::new(RecordingCellHook::default());
    install_async_cell_hook(hook.clone());
    let model = Arc::new(ModelArcSwap::new(0_u64));

    let workers = (1_u64..=16)
        .map(|value| {
            let model = Arc::clone(&model);
            std::thread::spawn(move || model.store(Arc::new(value)))
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().expect("store worker");
    }

    assert_eq!(*model.load_full(), **model.load());
    let events = hook.take();
    let stores = events
        .iter()
        .filter_map(|event| match event {
            CellEvent::Store(_, version) => Some(*version),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(stores, (1..=16).collect::<Vec<_>>());
    assert!(events
        .iter()
        .filter_map(|event| match event {
            CellEvent::Load(_, version) => Some(*version),
            _ => None,
        })
        .all(|version| version == 16));
    clear_async_cell_hook();
}

#[test]
fn cell_hook_is_additive_and_reports_created_store_load_order() {
    let _serial = serial();
    reset_model_async_ids_for_model();
    let hook = Arc::new(RecordingCellHook::default());
    install_async_cell_hook(hook.clone());

    let model = ModelArcSwap::new(7_u8);
    model.store(Arc::new(8));
    let _ = model.load_full();

    assert_eq!(
        hook.take(),
        vec![
            CellEvent::Created(1),
            CellEvent::Store(1, 1),
            CellEvent::Load(1, 1)
        ]
    );
    clear_async_cell_hook();
}
