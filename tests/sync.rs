use std::thread::{self, scope};
use std::time::Duration;

use loole::{bounded, SendError, SendTimeoutError};

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
fn send() {
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
fn send_timeout() {
    let (tx, rx) = bounded(2);
    scope(|scope| {
        scope.spawn(move || {
            assert_eq!(tx.send_timeout(1, ms(1000)), Ok(()));
            assert_eq!(tx.send_timeout(2, ms(1000)), Ok(()));
            assert_eq!(
                tx.send_timeout(3, ms(500)),
                Err(SendTimeoutError::Timeout(3))
            );
            thread::sleep(ms(1000));
            assert_eq!(tx.send_timeout(4, ms(1000)), Ok(()));
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
