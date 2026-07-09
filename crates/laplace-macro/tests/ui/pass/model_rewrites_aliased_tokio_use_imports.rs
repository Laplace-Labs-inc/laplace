// SPDX-License-Identifier: Apache-2.0
#![allow(unused_imports)]

#[laplace_macro::model]
fn uses_aliased_mpsc_channel() {
    use tokio::sync::mpsc;

    let (tx, mut rx) = mpsc::channel::<u8>(1);
    tx.try_send(7_u8).expect("try_send succeeds");
    assert_eq!(rx.try_recv().expect("try_recv succeeds"), 7);
}

#[laplace_macro::model]
fn uses_renamed_tokio_mutex() {
    use tokio::sync::Mutex as TMutex;

    let value: TMutex<u8> = TMutex::new(1_u8);
    let guard = value.try_lock().expect("uncontended try_lock succeeds");
    assert_eq!(*guard, 1);
}

#[laplace_macro::model]
fn uses_single_segment_channel_import() {
    use tokio::sync::mpsc::channel;

    let (tx, mut rx) = channel::<u8>(1);
    tx.try_send(9_u8).expect("try_send succeeds");
    assert_eq!(rx.try_recv().expect("try_recv succeeds"), 9);
}

fn main() {
    uses_aliased_mpsc_channel();
    uses_renamed_tokio_mutex();
    uses_single_segment_channel_import();
}
