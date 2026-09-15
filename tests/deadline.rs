use std::thread;
use std::time::{Duration, Instant};

use loole::{bounded, unbounded, RecvTimeoutError, SendTimeoutError};

fn ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

fn sync_sleep(ms: u64) {
    thread::sleep(Duration::from_millis(ms))
}

#[test]
fn send_deadline_ok() {
    let (tx, rx) = bounded(2);
    let deadline = Instant::now() + ms(1000);
    assert_eq!(tx.send_deadline(1, deadline), Ok(()));
    assert_eq!(rx.recv(), Ok(1));
}

#[test]
fn send_deadline_timeout() {
    let (tx, _rx) = bounded(1);
    tx.send(1).unwrap();
    let start = Instant::now();
    let deadline = start + ms(200);
    assert_eq!(tx.send_deadline(2, deadline), Err(SendTimeoutError::Timeout(2)));
    let elapsed = start.elapsed();
    assert!(ms(150) <= elapsed && elapsed < ms(1500), "elapsed: {elapsed:?}");
}

#[test]
fn send_deadline_disconnect() {
    let (tx, rx) = bounded(1);
    tx.send(1).unwrap();
    drop(rx);
    let deadline = Instant::now() + ms(1000);
    assert_eq!(tx.send_deadline(2, deadline), Err(SendTimeoutError::Disconnected(2)));
}

#[test]
fn send_deadline_handoff_when_receiver_is_ready() {
    let (tx, rx) = bounded(0);
    let h = thread::spawn(move || rx.recv());
    sync_sleep(100);
    let deadline = Instant::now() + ms(1000);
    assert_eq!(tx.send_deadline(1, deadline), Ok(()));
    assert_eq!(h.join().unwrap(), Ok(1));
}

#[test]
fn recv_deadline_disconnect() {
    let (tx, rx) = bounded::<()>(1);
    drop(tx);
    let deadline = Instant::now() + ms(1000);
    assert_eq!(rx.recv_deadline(deadline), Err(RecvTimeoutError::Disconnected));
}

#[test]
fn recv_deadline_message_already_buffered() {
    let (tx, rx) = bounded(2);
    tx.send(1).unwrap();
    let deadline = Instant::now() + ms(1000);
    assert_eq!(rx.recv_deadline(deadline), Ok(1));
}

#[test]
fn recv_deadline_timeout() {
    let (_tx, rx) = bounded::<()>(1);
    let start = Instant::now();
    let deadline = start + ms(200);
    assert_eq!(rx.recv_deadline(deadline), Err(RecvTimeoutError::Timeout));
    let elapsed = start.elapsed();
    assert!(ms(150) <= elapsed && elapsed < ms(1500), "elapsed: {elapsed:?}");
}

#[test]
fn recv_deadline_handoff_when_sender_is_ready() {
    let (tx, rx) = bounded(0);
    let h = thread::spawn(move || tx.send(1));
    sync_sleep(100);
    let deadline = Instant::now() + ms(1000);
    assert_eq!(rx.recv_deadline(deadline), Ok(1));
    assert_eq!(h.join().unwrap(), Ok(()));
}

#[test]
fn send_timeout_zero_on_full_channel() {
    let (tx, _rx) = bounded(1);
    tx.send(1).unwrap();
    let start = Instant::now();
    assert_eq!(
        tx.send_timeout(2, Duration::ZERO),
        Err(SendTimeoutError::Timeout(2))
    );
    assert!(start.elapsed() < ms(1000), "zero timeout should return immediately");
}

#[test]
fn send_timeout_zero_on_rendezvous_channel() {
    let (tx, _rx) = bounded(0);
    assert_eq!(
        tx.send_timeout(1, Duration::ZERO),
        Err(SendTimeoutError::Timeout(1))
    );
}

#[test]
fn recv_timeout_zero_on_empty_channel() {
    let (_tx, rx) = unbounded::<()>();
    let start = Instant::now();
    assert_eq!(rx.recv_timeout(Duration::ZERO), Err(RecvTimeoutError::Timeout));
    assert!(start.elapsed() < ms(1000), "zero timeout should return immediately");
}

#[test]
fn send_deadline_in_the_past_returns_timeout() {
    // A deadline already in the past behaves like a zero timeout: the send
    // fails immediately with `Timeout` instead of panicking.
    let (tx, _rx) = bounded(1);
    tx.send(1).unwrap();
    let start = Instant::now();
    assert_eq!(
        tx.send_deadline(2, Instant::now() - ms(100)),
        Err(SendTimeoutError::Timeout(2))
    );
    assert!(start.elapsed() < ms(1000), "past deadline should return immediately");
}

#[test]
fn send_deadline_in_the_past_still_succeeds_when_space_available() {
    // A past deadline does not prevent an immediately-completable send.
    let (tx, rx) = bounded(1);
    assert_eq!(tx.send_deadline(1, Instant::now() - ms(100)), Ok(()));
    assert_eq!(rx.recv(), Ok(1));
}

#[test]
fn recv_deadline_in_the_past_returns_timeout() {
    // A deadline already in the past behaves like a zero timeout: a buffered
    // message is still returned, otherwise `Timeout` — no panic.
    let (_tx, rx) = bounded::<()>(1);
    let start = Instant::now();
    assert_eq!(
        rx.recv_deadline(Instant::now() - ms(100)),
        Err(RecvTimeoutError::Timeout)
    );
    assert!(start.elapsed() < ms(1000), "past deadline should return immediately");
}

#[test]
fn recv_deadline_in_the_past_still_returns_buffered_message() {
    let (tx, rx) = bounded(1);
    tx.send(1).unwrap();
    assert_eq!(rx.recv_deadline(Instant::now() - ms(100)), Ok(1));
}

#[test]
fn send_timeout_huge_duration_does_not_panic() {
    // `Instant::now() + timeout` overflows for extreme durations; the send
    // must wait (effectively) forever instead of panicking.
    let (tx, rx) = bounded(1);
    tx.send(1).unwrap(); // full: the send below parks
    let h = thread::spawn(move || tx.send_timeout(2, Duration::MAX));
    sync_sleep(100);
    assert_eq!(rx.recv(), Ok(1));
    assert_eq!(h.join().unwrap(), Ok(()));
    assert_eq!(rx.recv(), Ok(2));
}
