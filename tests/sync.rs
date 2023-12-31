use std::thread::{self, scope};
use std::time::{Duration, Instant};

use loole::{bounded, RecvError, SendError};

fn ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

fn sync_sleep(ms: u64) {
    thread::sleep(Duration::from_millis(ms))
}

#[test]
fn sync_send_before_recv_buffer_0() {
    let (tx, rx) = bounded(0);
    let h = thread::spawn(move || tx.send(1));
    sync_sleep(100);
    assert_eq!(rx.recv(), Ok(1));
    assert_eq!(h.join().unwrap(), Ok(()));
}

#[test]
fn sync_recv_before_send_buffer_0() {
    let (tx, rx) = bounded(0);
    let h = thread::spawn(move || rx.recv());
    sync_sleep(100);
    assert_eq!(tx.send(1), Ok(()));
    assert_eq!(h.join().unwrap(), Ok(1));
}

#[test]
fn sync_send_before_recv_buffer_1() {
    let (tx, rx) = bounded(1);
    assert_eq!(tx.send(1), Ok(()));
    assert_eq!(rx.recv(), Ok(1));
}

#[test]
fn sync_recv_before_send_buffer_1() {
    let (tx, rx) = bounded(1);
    let h = thread::spawn(move || rx.recv());
    sync_sleep(100);
    assert_eq!(tx.send(1), Ok(()));
    assert_eq!(h.join().unwrap(), Ok(1));
}

#[test]
fn sync_recv_after_manually_closed_sender() {
    let (tx, rx) = bounded(1);
    assert_eq!(tx.send(1), Ok(()));
    assert!(tx.close());
    assert_eq!(rx.recv(), Ok(1));
    assert_eq!(rx.recv(), Err(RecvError::Disconnected));
}

#[test]
fn sync_recv_after_manually_closeed_receiver() {
    let (tx, rx) = bounded(1);
    assert_eq!(tx.send(1), Ok(()));
    assert!(rx.close());
    assert_eq!(rx.recv(), Ok(1));
    assert_eq!(rx.recv(), Err(RecvError::Disconnected));
}

#[test]
fn sync_is_closed_closed_by_sender_drop() {
    let (tx, rx) = bounded::<()>(1);
    assert!(!rx.is_closed());
    drop(tx);
    assert!(rx.is_closed());
}

#[test]
fn sync_is_closed_closed_by_receiver_drop() {
    let (tx, rx) = bounded::<()>(1);
    assert!(!tx.is_closed());
    drop(rx);
    assert!(tx.is_closed());
}

#[test]
fn sync_2_sends_before_2_recvs_buffer_1() {
    let (tx, rx) = bounded(1);
    assert_eq!(tx.capacity(), Some(1));
    assert_eq!(tx.len(), 0);
    tx.send(1).unwrap();
    assert_eq!(tx.len(), 1);
    thread::spawn(move || tx.send(2));
    sync_sleep(100);
    assert_eq!(rx.len(), 1);
    let r1r = rx.recv();
    assert_eq!(r1r, Ok(1));
    assert_eq!(rx.len(), 1);
    let r2r = rx.recv();
    assert_eq!(r2r, Ok(2));
    assert_eq!(rx.len(), 0);
}

#[test]
fn sync_send() {
    let (tx, rx) = bounded(2);
    scope(|scope| {
        scope.spawn(move || {
            assert_eq!(tx.send(1), Ok(()));
            assert_eq!(tx.send(2), Ok(()));
            thread::sleep(ms(1500));
            assert_eq!(tx.send(4), Ok(()));
            thread::sleep(ms(1000));
            assert_eq!(tx.send(5), Err(SendError(5)));
        });
        scope.spawn(move || {
            thread::sleep(ms(1000));
            assert_eq!(rx.recv(), Ok(1));
            thread::sleep(ms(1000));
            assert_eq!(rx.recv(), Ok(2));
            assert_eq!(rx.recv(), Ok(4));
        });
    });
}

#[test]
fn sync_shift_pending_send_buffer_0() {
    let (tx, rx) = bounded(0);
    let tx_clone = tx.clone();
    scope(|scope| {
        scope.spawn(move || {
            let start = Instant::now();
            assert_eq!(tx.send(1), Ok(()));
            let elapsed = start.elapsed();
            assert!(ms(900) <= elapsed && elapsed <= ms(1100));
        });
        scope.spawn(move || {
            let start = Instant::now();
            thread::sleep(ms(100));
            assert_eq!(tx_clone.send(2), Ok(()));
            let elapsed = start.elapsed();
            assert!(ms(1900) <= elapsed && elapsed <= ms(2100));
        });
        scope.spawn(move || {
            thread::sleep(ms(1000));
            assert_eq!(rx.recv(), Ok(1));
            thread::sleep(ms(1000));
            assert_eq!(rx.recv(), Ok(2));
        });
    });
}

#[test]
fn sync_shift_pending_send_buffer_2() {
    let (tx, rx) = bounded(2);
    scope(|scope| {
        assert_eq!(tx.send(1), Ok(()));
        assert_eq!(tx.send(2), Ok(()));
        scope.spawn(move || {
            let start = Instant::now();
            assert_eq!(tx.send(3), Ok(()));
            let elapsed = start.elapsed();
            assert!(ms(900) <= elapsed && elapsed <= ms(1100));
        });
        scope.spawn(move || {
            thread::sleep(ms(1000));
            assert_eq!(rx.recv(), Ok(1));
        });
    });
}

#[test]
fn sync_drain() {
    let (tx, rx) = bounded(2);
    let tx_clone_1 = tx.clone();
    let tx_clone_2 = tx.clone();
    scope(|scope| {
        scope.spawn(move || {
            assert_eq!(tx.send(1), Ok(()));
            assert_eq!(tx.send(2), Ok(()));
        });
        scope.spawn(move || {
            let start = Instant::now();
            thread::sleep(ms(100));
            assert_eq!(tx_clone_1.send(3), Ok(()));
            let elapsed = start.elapsed();
            assert!(ms(900) <= elapsed && elapsed <= ms(1100));
        });
        scope.spawn(move || {
            let start = Instant::now();
            thread::sleep(ms(100));
            assert_eq!(tx_clone_2.send(4), Ok(()));
            let elapsed = start.elapsed();
            assert!(ms(900) <= elapsed && elapsed <= ms(1100));
        });
        scope.spawn(move || {
            thread::sleep(ms(1000));
            assert_eq!(rx.len(), 2);
            let v = rx.drain().collect::<Vec<_>>();
            assert_eq!(v, [1, 2]);
            assert_eq!(rx.len(), 2);
        });
    });
}
