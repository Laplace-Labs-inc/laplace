// SPDX-License-Identifier: Apache-2.0
//! Shared BYOC Direction A program body.

use parking_lot::Mutex;
use parking_lot::RwLock;
use std::sync::Arc;

pub const AB_BA_RESOURCES: usize = 2;
pub const FAN_OUT_RESOURCES: usize = 2;
pub const CROSSBEAM_CHANNEL_RESOURCES: usize = 2;

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

/// AB-BA deadlock over annotated `std::sync::RwLock` write locks.
///
/// `#[laplace::model]` rewrites `std::sync::RwLock` to `laplace_sdk::rt::ModelRwLock`,
/// whose `write` acquisitions route through the same exclusive engine boundary
/// as `ModelMutex`. The two write locks are grabbed in opposite orders, so the
/// engine must prove the classic circular-wait deadlock (P-3 coverage).
#[laplace::model]
pub fn std_sync_rwlock_ab_ba_program() {
    let left = Arc::new(std::sync::RwLock::new(()));
    let right = Arc::new(std::sync::RwLock::new(()));

    let left_first = Arc::clone(&left);
    let right_second = Arc::clone(&right);
    let thread0 = std::thread::spawn(move || {
        let _left = left_first.write().expect("left write");
        let _right = right_second.write().expect("right write");
    });

    let right_first = Arc::clone(&right);
    let left_second = Arc::clone(&left);
    let thread1 = std::thread::spawn(move || {
        let _right = right_first.write().expect("right write");
        let _left = left_second.write().expect("left write");
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

pub fn crossbeam_channel_recv_cycle_program<S>(spawn: S)
where
    S: Fn(usize, Box<dyn FnOnce() + Send + 'static>),
{
    let (send_a, recv_a) = crossbeam_channel::bounded::<usize>(1);
    let (send_b, recv_b) = crossbeam_channel::bounded::<usize>(1);

    spawn(
        0,
        Box::new(move || {
            let _keep_sender_to_b = send_b;
            let _ = recv_a.recv();
        }),
    );

    spawn(
        1,
        Box::new(move || {
            let _keep_sender_to_a = send_a;
            let _ = recv_b.recv();
        }),
    );
}

pub fn crossbeam_channel_bounded_full_send_cycle_program<S>(spawn: S)
where
    S: Fn(usize, Box<dyn FnOnce() + Send + 'static>),
{
    let (send_a, recv_a) = crossbeam_channel::bounded::<usize>(1);
    let (send_b, recv_b) = crossbeam_channel::bounded::<usize>(1);
    send_a.send(1).expect("prefill channel a");
    send_b.send(2).expect("prefill channel b");

    spawn(
        0,
        Box::new(move || {
            let _keep_receiver_a = recv_a;
            let _ = send_b.send(3);
        }),
    );

    spawn(
        1,
        Box::new(move || {
            let _keep_receiver_b = recv_b;
            let _ = send_a.send(4);
        }),
    );
}

pub fn crossbeam_channel_all_senders_drop_is_clean_program<S>(spawn: S)
where
    S: Fn(usize, Box<dyn FnOnce() + Send + 'static>),
{
    let (sender, receiver) = crossbeam_channel::bounded::<usize>(1);
    drop(sender);

    spawn(
        0,
        Box::new(move || {
            assert!(receiver.recv().is_err());
        }),
    );
}
