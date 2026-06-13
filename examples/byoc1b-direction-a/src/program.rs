// SPDX-License-Identifier: Apache-2.0
//! Shared BYOC Direction A program body.

use parking_lot::Mutex;
use std::sync::Arc;

pub const AB_BA_RESOURCES: usize = 2;

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
