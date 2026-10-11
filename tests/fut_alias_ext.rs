//! RED spec for the second fut-alias extension (zenoh follow-ups).
//!
//! Context: zenoh wraps loole's sync iterators in `#[derive(Debug)]` structs
//! (`fifo.rs:199-242`, E0277 today — flume implements `Debug` for all three),
//! calls `RecvStream::is_disconnected()` (`fifo.rs:311`, E0599 today — flume
//! `async.rs` has it), and calls `FusedStream::is_terminated` on the stream
//! (`fifo.rs:350`, E0277 today — flume `async.rs:588-592`).
//!
//! These tests currently FAIL TO COMPILE — that is the correct RED state.
//! DO NOT "fix" them by weakening assertions.
use std::time::Duration;

use futures::stream::FusedStream;
use loole::unbounded;

/// flume implements `Debug` on `Iter` / `TryIter` / `IntoIter`; zenoh's
/// `#[derive(Debug)]` wrappers require it.
#[test]
fn sync_iters_are_debug() {
    let (_tx, rx) = unbounded::<u32>();
    let iter_dbg = format!("{:?}", rx.iter());
    let try_iter_dbg = format!("{:?}", rx.try_iter());
    assert!(iter_dbg.contains("Iter"), "got: {iter_dbg}");
    assert!(try_iter_dbg.contains("TryIter"), "got: {try_iter_dbg}");

    let (_tx2, rx2) = unbounded::<u32>();
    let into_iter_dbg = format!("{:?}", rx2.into_iter());
    assert!(into_iter_dbg.contains("IntoIter"), "got: {into_iter_dbg}");
}

/// flume `RecvStream::is_disconnected()`: false while a sender lives, true
/// once the last sender drops.
#[test]
fn recv_stream_is_disconnected() {
    let (tx, rx) = unbounded::<u32>();
    let s = rx.stream();
    assert!(!s.is_disconnected());
    drop(tx);
    assert!(s.is_disconnected());
}

/// flume `FusedStream for RecvStream` delegates to the inner future:
/// `disconnected && empty`. Live → false; closed+queued → false (still
/// receivable); drained+closed → true.
#[test]
fn recv_stream_fused_termination_matches_flume() {
    let (tx, rx) = unbounded::<u32>();
    let s = rx.stream();
    assert!(!FusedStream::is_terminated(&s));

    tx.send(1).unwrap();
    drop(tx);
    assert!(s.is_disconnected());
    assert!(!FusedStream::is_terminated(&s));

    assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(1));
    assert!(FusedStream::is_terminated(&s));
}
