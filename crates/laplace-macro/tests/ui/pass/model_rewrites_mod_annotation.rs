// SPDX-License-Identifier: Apache-2.0
#![allow(unused_imports)]

#[laplace_macro::model]
mod target {
    use tokio::sync::mpsc;

    pub fn make_channel_pair() {
        let (tx, mut rx) = mpsc::channel::<u8>(1);
        tx.try_send(3_u8).expect("try_send succeeds");
        assert_eq!(rx.try_recv().expect("try_recv succeeds"), 3);
    }

    pub fn make_second_channel_pair() {
        let (tx, mut rx) = mpsc::channel::<u8>(4);
        tx.try_send(5_u8).expect("try_send succeeds");
        assert_eq!(rx.try_recv().expect("try_recv succeeds"), 5);
    }
}

fn main() {
    target::make_channel_pair();
    target::make_second_channel_pair();
}
