// SPDX-License-Identifier: Apache-2.0
//! Shared BYOC Direction A program body.

use parking_lot::Mutex;
use parking_lot::RwLock;
use std::sync::Arc;

pub const AB_BA_RESOURCES: usize = 2;
pub const FAN_OUT_RESOURCES: usize = 2;

#[laplace::model]
pub fn std_spawn_mutex_ab_ba_program() {
    let left = Arc::new(Mutex::new(()));
    let right = Arc::new(Mutex::new(()));

    let left_first = Arc::clone(&left);
    let right_second = Arc::clone(&right);
    let thread0 = std::thread::spawn(move || {
        let _left = left_first.lock();
        let _right = right_second.lock();
    });

    let right_first = Arc::clone(&right);
    let left_second = Arc::clone(&left);
    let thread1 = std::thread::spawn(move || {
        let _right = right_first.lock();
        let _left = left_second.lock();
    });

    thread0.join().expect("thread0 completes");
    thread1.join().expect("thread1 completes");
}

#[laplace::model]
pub fn std_sync_mutex_ab_ba_program() {
    let left = Arc::new(std::sync::Mutex::new(()));
    let right = Arc::new(std::sync::Mutex::new(()));

    let left_first = Arc::clone(&left);
    let right_second = Arc::clone(&right);
    let thread0 = std::thread::spawn(move || {
        let _left = left_first.lock().expect("left lock");
        let _right = right_second.lock().expect("right lock");
    });

    let right_first = Arc::clone(&right);
    let left_second = Arc::clone(&left);
    let thread1 = std::thread::spawn(move || {
        let _right = right_first.lock().expect("right lock");
        let _left = left_second.lock().expect("left lock");
    });

    thread0.join().expect("thread0 completes");
    thread1.join().expect("thread1 completes");
}

pub fn parking_lot_mutex_ab_ba_program<S>(spawn: S)
where
    S: Fn(usize, Box<dyn FnOnce() + Send + 'static>),
{
    let left = Arc::new(Mutex::new(()));
    let right = Arc::new(Mutex::new(()));

    let left_first = Arc::clone(&left);
    let right_second = Arc::clone(&right);
    spawn(
        0,
        Box::new(move || {
            let _left = left_first.lock();
            let _right = right_second.lock();
        }),
    );

    let right_first = Arc::clone(&right);
    let left_second = Arc::clone(&left);
    spawn(
        1,
        Box::new(move || {
            let _right = right_first.lock();
            let _left = left_second.lock();
        }),
    );
}

pub fn parking_lot_rwlock_ab_ba_program<S>(spawn: S)
where
    S: Fn(usize, Box<dyn FnOnce() + Send + 'static>),
{
    let left = Arc::new(RwLock::new(()));
    let right = Arc::new(RwLock::new(()));

    let left_first = Arc::clone(&left);
    let right_second = Arc::clone(&right);
    spawn(
        0,
        Box::new(move || {
            let _left = left_first.write();
            let _right = right_second.write();
        }),
    );

    let right_first = Arc::clone(&right);
    let left_second = Arc::clone(&left);
    spawn(
        1,
        Box::new(move || {
            let _right = right_first.write();
            let _left = left_second.write();
        }),
    );
}

pub fn parking_lot_rwlock_read_read_ab_ba_program<S>(spawn: S)
where
    S: Fn(usize, Box<dyn FnOnce() + Send + 'static>),
{
    let left = Arc::new(RwLock::new(()));
    let right = Arc::new(RwLock::new(()));

    let left_first = Arc::clone(&left);
    let right_second = Arc::clone(&right);
    spawn(
        0,
        Box::new(move || {
            let _left = left_first.read();
            let _right = right_second.read();
        }),
    );

    let right_first = Arc::clone(&right);
    let left_second = Arc::clone(&left);
    spawn(
        1,
        Box::new(move || {
            let _right = right_first.read();
            let _left = left_second.read();
        }),
    );
}

pub fn parking_lot_rwlock_read_write_ab_ba_program<S>(spawn: S)
where
    S: Fn(usize, Box<dyn FnOnce() + Send + 'static>),
{
    let left = Arc::new(RwLock::new(()));
    let right = Arc::new(RwLock::new(()));

    let left_first = Arc::clone(&left);
    let right_second = Arc::clone(&right);
    spawn(
        0,
        Box::new(move || {
            let _left = left_first.write();
            let _right = right_second.read();
        }),
    );

    let right_first = Arc::clone(&right);
    let left_second = Arc::clone(&left);
    spawn(
        1,
        Box::new(move || {
            let _right = right_first.write();
            let _left = left_second.read();
        }),
    );
}

pub fn parking_lot_rwlock_multi_reader_fan_out_program<S>(spawn: S)
where
    S: Fn(usize, Box<dyn FnOnce() + Send + 'static>),
{
    let readers_resource = Arc::new(RwLock::new(()));
    let writer_resource = Arc::new(RwLock::new(()));

    let reader0_first = Arc::clone(&readers_resource);
    let reader0_second = Arc::clone(&writer_resource);
    spawn(
        0,
        Box::new(move || {
            let _shared = reader0_first.read();
            let _blocked_by_writer = reader0_second.write();
        }),
    );

    let reader1_first = Arc::clone(&readers_resource);
    let reader1_second = Arc::clone(&writer_resource);
    spawn(
        1,
        Box::new(move || {
            let _shared = reader1_first.read();
            let _blocked_by_writer = reader1_second.write();
        }),
    );

    let writer_first = Arc::clone(&writer_resource);
    let writer_second = Arc::clone(&readers_resource);
    spawn(
        2,
        Box::new(move || {
            let _exclusive = writer_first.write();
            let _blocked_by_readers = writer_second.write();
        }),
    );
}
