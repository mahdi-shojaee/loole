//! Trait bounds and cancellation contract of `SendFuture` / `RecvFuture`.
//!
//! Context: downstream crates (zenoh ×6, songbird, sea-streamer ×3,
//! quic-rpc) store, pin, and delegate through these futures (see
//! `fut_alias_red.rs`). Every test below locks the bounds and contracts
//! those positions rely on.
//!
//! Break each test catches (mutation check):
//! - `*_is_unpin`: removing the `Unpin` impl breaks quic-rpc's
//!   `Pin::new(&mut fut).poll()` and sea-streamer's `poll_unpin`.
//! - `*_is_send`: losing `Send` breaks storing futures in structs moved
//!   across tasks (zenoh handlers, quic-rpc transport state).
//! - `*_debug_*`: losing `Debug` breaks `#[derive(Debug)]` wrappers
//!   (zenoh's `RecvFut` struct derives `Debug`).
//! - `drop_pending_*`: losing cancel-safety reintroduces the concurrent-forward
//!   class of delivery loss (message stuck or duplicated after drop).
//! - `terminated_predicate_*`: the truth table `FusedFuture::is_terminated`
//!   must implement (flume `src/async.rs`: recv `disconnected && empty`,
//!   send `disconnected`). If the channel-state inputs drift, the RED
//!   test's predicate goes wrong with them.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::FutureExt as _;
use loole::{bounded, unbounded, RecvError, SendError};

fn assert_send<T: Send>() {}
fn assert_unpin<T: Unpin>() {}

// ---------------------------------------------------------------------------
// Bounds: Unpin
// ---------------------------------------------------------------------------

/// `Pin::new(&mut fut)` must work: quic-rpc polls stored futures exactly
/// this way (`src/transport/flume.rs:161,189`).
#[test]
fn send_future_is_unpin() {
    let (tx, _rx) = bounded::<u32>(1);
    let mut f = tx.send_async(1);
    let cx = &mut Context::from_waker(futures::task::noop_waker_ref());
    assert!(Pin::new(&mut f).poll(cx).is_ready());
    assert_unpin::<loole::SendFuture<'_, u32>>();
}

/// sea-streamer polls via `FutureExt::poll_unpin`
/// (`sea-streamer-redis/src/consumer/future.rs`).
#[test]
fn recv_future_is_unpin() {
    let (tx, rx) = unbounded::<u32>();
    tx.send(7).unwrap();
    let mut f = rx.recv_async();
    let cx = &mut Context::from_waker(futures::task::noop_waker_ref());
    assert_eq!(f.poll_unpin(cx), Poll::Ready(Ok(7)));
    assert_unpin::<loole::RecvFuture<'_, u32>>();
}

// ---------------------------------------------------------------------------
// Bounds: Send
// ---------------------------------------------------------------------------

/// Futures are stored in structs handed across threads (zenoh FIFO handlers,
/// quic-rpc transport state) — they must be `Send`.
#[test]
fn send_future_is_send() {
    // Compile-time contract, borrowed and owned forms.
    assert_send::<loole::SendFuture<'_, u32>>();
    assert_send::<loole::SendFuture<'static, u32>>();
    // Real cross-thread move: an owned future polled on another thread.
    let (tx, rx) = bounded::<u32>(1);
    let f = tx.into_send_async(2);
    let h = std::thread::spawn(move || {
        let out: Result<(), SendError<u32>> = async_std::task::block_on(f);
        assert_eq!(out, Ok(()));
    });
    h.join().unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(2));
}

/// Same for the receive side.
#[test]
fn recv_future_is_send() {
    assert_send::<loole::RecvFuture<'_, u32>>();
    assert_send::<loole::RecvFuture<'static, u32>>();
    let (tx, rx) = unbounded::<u32>();
    tx.send(11).unwrap();
    drop(tx);
    let f = rx.into_recv_async();
    let h = std::thread::spawn(move || {
        let out: Result<u32, RecvError> = async_std::task::block_on(f);
        assert_eq!(out, Ok(11));
    });
    h.join().unwrap();
}

// ---------------------------------------------------------------------------
// Bounds: Debug + output types
// ---------------------------------------------------------------------------

/// zenoh's wrapper `#[derive(Debug)] pub struct RecvFut(flume::RecvFut)`
/// requires the inner future to be `Debug`.
#[test]
fn futures_are_debug() {
    let (tx, rx) = unbounded::<u32>();
    let send_dbg = format!("{:?}", tx.send_async(1));
    let recv_dbg = format!("{:?}", rx.recv_async());
    assert!(send_dbg.contains("SendFuture"), "got: {send_dbg}");
    assert!(recv_dbg.contains("RecvFuture"), "got: {recv_dbg}");
}

/// Output types must match flume exactly: `Result<(), SendError<T>>` and
/// `Result<T, RecvError>`, with the message preserved in the error.
#[test]
fn future_output_types_match_flume() {
    let (tx, rx) = unbounded::<u32>();
    let out: Result<(), SendError<u32>> =
        async_std::task::block_on(tx.send_async(41));
    assert_eq!(out, Ok(()));
    let out: Result<u32, RecvError> = async_std::task::block_on(rx.recv_async());
    assert_eq!(out, Ok(41));

    // No receiver: the send resolves with the message back, like flume.
    let (tx, rx) = unbounded::<u32>();
    drop(rx);
    let out = async_std::task::block_on(tx.send_async(9));
    assert_eq!(out, Err(SendError(9)));

    // No sender: the recv resolves disconnected, like flume.
    let (tx, rx) = unbounded::<u32>();
    drop(tx);
    let out = async_std::task::block_on(rx.recv_async());
    assert_eq!(out, Err(RecvError::Disconnected));
}

// ---------------------------------------------------------------------------
// Cancellation: dropping a pending future loses nothing and duplicates nothing
// ---------------------------------------------------------------------------

/// Drop a parked send: the message must NOT be delivered later (flume
/// `SendFut::reset_hook` contract). A fresh send then succeeds normally.
#[test]
fn drop_pending_send_future_cancels_without_loss_or_duplication() {
    let (tx, rx) = bounded::<u32>(1);
    tx.send(1).unwrap();
    let cx = &mut Context::from_waker(futures::task::noop_waker_ref());
    {
        let mut f = tx.send_async(2);
        assert!(Pin::new(&mut f).poll(cx).is_pending());
        // Dropped while parked: message 2 is cancelled.
    }
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(1));
    assert_eq!(tx.try_send(3), Ok(()));
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(3));
}

/// Drop a parked recv: the message must still be receivable by another
/// `recv_async` (flume `RecvFut::reset_hook` wakes a replacement waiter).
#[test]
fn drop_pending_recv_future_preserves_message() {
    let (tx, rx) = unbounded::<u32>();
    let cx = &mut Context::from_waker(futures::task::noop_waker_ref());
    {
        let mut f = rx.recv_async();
        assert!(Pin::new(&mut f).poll(cx).is_pending());
        // Dropped while parked.
    }
    tx.send(5).unwrap();
    let out = async_std::task::block_on(rx.recv_async());
    assert_eq!(out, Ok(5));
}

// ---------------------------------------------------------------------------
// Termination-predicate ingredients (what FusedFuture::is_terminated needs)
// ---------------------------------------------------------------------------

/// flume `RecvFut::is_terminated == disconnected && empty`: closed but
/// non-empty is NOT terminated (queued messages stay receivable).
#[test]
fn recv_terminated_predicate_inputs() {
    let (tx, rx) = unbounded::<u32>();
    // Live channel: not terminated.
    assert!(!rx.is_disconnected());
    // Closed with a queued message: still NOT terminated.
    tx.send(1).unwrap();
    drop(tx);
    assert!(rx.is_disconnected());
    assert!(!rx.is_empty());
    // Drained and closed: terminated.
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(1));
    assert!(rx.is_disconnected());
    assert!(rx.is_empty());
}

/// flume `SendFut::is_terminated == disconnected`: live sender is not
/// terminated; last receiver dropped means terminated.
#[test]
fn send_terminated_predicate_inputs() {
    let (tx, rx) = bounded::<u32>(1);
    assert!(!tx.is_closed());
    drop(rx);
    assert!(tx.is_closed());
}
