#![deny(clippy::all, clippy::pedantic)]

//! Basic DashMap test without probing to verify TrackedParkingLotRwLock works

use dashmap::DashMap;
use std::sync::Arc;

#[test]
fn dashmap_basic_operations() {
    let map = Arc::new(DashMap::new());

    // Single-threaded basic operations
    map.insert("a".to_string(), 1i64);
    let val = map.get("a").unwrap();
    assert_eq!(*val, 1i64);
    drop(val);

    map.insert("b".to_string(), 2i64);
    let val = map.get("b").unwrap();
    assert_eq!(*val, 2i64);
    drop(val);

    assert_eq!(map.len(), 2);
    println!("[dashmap-basic] Single-threaded operations: PASS");
}

#[test]
fn dashmap_concurrent_basic() {
    let map = Arc::new(DashMap::new());

    let mut handles = vec![];

    for tid in 0..2 {
        let m = map.clone();
        handles.push(std::thread::spawn(move || {
            m.insert(format!("key_{}", tid), tid as i64);
            let val = m.get(&format!("key_{}", tid)).unwrap();
            assert_eq!(*val, tid as i64);
        }));
    }

    for h in handles {
        h.join().expect("thread panicked");
    }

    assert_eq!(map.len(), 2);
    println!("[dashmap-basic] Concurrent operations: PASS");
}
