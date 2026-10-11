//! Contract spec for loole's channel futures (`SendFuture` / `RecvFuture`).
//!
//! Context: downstream crates use these futures in stored, pinned, and
//! delegated positions (zenoh wraps the recv future in its own struct AND
//! calls `FusedFuture::is_terminated` on it; songbird stores it under
//! `#[pin] Timeout<_>`; sea-streamer polls via `poll_unpin`; quic-rpc stores
//! AND polls both futures). Migrators coming from flume's
//! `r#async::{SendFut, RecvFut}` spellings map them with a single import
//! (`use loole::RecvFuture as RecvFut;`); every test below uses loole's
//! canonical root-type names only.
//!
//! Break each test catches (mutation check):
//! - `future_types_resolve`: the futures failing to resolve in argument
//!   position breaks every downstream signature using them.
//! - `recv_is_terminated_*` / `send_is_terminated_*`: wrong termination
//!   predicate hangs fused polling or ends streams early/late.
//! - `send_fut_has_is_disconnected`: the method missing breaks callers
//!   polling liveness off the send future.
//! - `downstream_pattern_*`: the exact downstream usage shapes failing to
//!   compile or misbehaving (zenoh delegation, quic-rpc stored polling,
//!   sea-streamer `poll_unpin`).

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::future::FusedFuture;
use loole::{RecvFuture, SendFuture, bounded, unbounded};

// ---------------------------------------------------------------------------
// Type resolution
// ---------------------------------------------------------------------------

/// `recv_async` / `send_async` yield the root future types, usable in
/// argument position with no friction.
#[test]
fn future_types_resolve() {
    fn takes_recv<T>(_: RecvFuture<'_, T>) {}
    fn takes_send<T>(_: SendFuture<'_, T>) {}
    let (tx, rx) = unbounded::<u32>();
    takes_recv(rx.recv_async());
    takes_send(tx.send_async(1));
}

// ---------------------------------------------------------------------------
// FusedFuture: termination predicates matching flume's semantics
// ---------------------------------------------------------------------------

/// Terminated once disconnected AND empty
/// (`flume 0.12.0 src/async.rs:518-522`).
#[test]
fn recv_is_terminated_matches_flume() {
    let (tx, rx) = unbounded::<u32>();
    // Live channel: not terminated.
    let f = rx.recv_async();
    assert!(!f.is_terminated());
    drop(f);
    // Closed with a queued message: NOT terminated (still receivable).
    tx.send(1).unwrap();
    drop(tx);
    let f = rx.recv_async();
    assert!(!f.is_terminated());
    drop(f);
    // Drained and closed: terminated.
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(1));
    let f = rx.recv_async();
    assert!(f.is_terminated());
}

/// Terminated once disconnected
/// (`flume 0.12.0 src/async.rs:256-260`).
#[test]
fn send_is_terminated_matches_flume() {
    let (tx, rx) = bounded::<u32>(1);
    // Live sender: not terminated.
    let f = tx.send_async(1);
    assert!(!f.is_terminated());
    drop(f);
    // Last receiver dropped: terminated.
    drop(rx);
    let f = tx.send_async(2);
    assert!(f.is_terminated());
}

// ---------------------------------------------------------------------------
// Method surface
// ---------------------------------------------------------------------------

/// `SendFuture::is_disconnected()` reports liveness off the future itself.
#[test]
fn send_fut_has_is_disconnected() {
    let (tx, rx) = bounded::<u32>(1);
    let f = tx.send_async(1);
    assert!(!f.is_disconnected());
    drop(f);
    drop(rx);
    let f = tx.send_async(2);
    assert!(f.is_disconnected());
}

// ---------------------------------------------------------------------------
// Downstream usage patterns, verbatim shapes
// ---------------------------------------------------------------------------

/// zenoh `fifo.rs:274-286`: own wrapper delegating `Future` + `FusedFuture`
/// to the inner future.
#[test]
fn downstream_pattern_zenoh_fused_delegation() {
    #[derive(Debug)]
    struct ZenohRecvFut<'a, T>(RecvFuture<'a, T>);

    impl<T> Future for ZenohRecvFut<'_, T> {
        type Output = Result<T, loole::RecvError>;
        fn poll(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<Self::Output> {
            Future::poll(Pin::new(&mut self.0), cx)
        }
    }

    impl<T> FusedFuture for ZenohRecvFut<'_, T> {
        fn is_terminated(&self) -> bool {
            FusedFuture::is_terminated(&self.0)
        }
    }

    let (tx, rx) = unbounded::<u32>();
    let w = ZenohRecvFut(rx.recv_async());
    assert!(!w.is_terminated());
    tx.send(3).unwrap();
    let out: Result<u32, loole::RecvError> = async_std::task::block_on(w);
    assert_eq!(out, Ok(3));
}

/// quic-rpc `flume.rs:135,157-161`: struct storing the future, polled via
/// `Pin::new(&mut ...)`.
#[test]
fn downstream_pattern_quic_rpc_stored_poll() {
    struct StoredSend<'a> {
        inner: SendFuture<'a, u32>,
    }
    struct StoredRecv<'a> {
        wrapped: RecvFuture<'a, u32>,
    }

    let (tx, rx) = unbounded::<u32>();
    let mut s = StoredSend {
        inner: tx.send_async(8),
    };
    let mut r = StoredRecv {
        wrapped: rx.recv_async(),
    };
    let cx = &mut Context::from_waker(futures::task::noop_waker_ref());
    assert!(Pin::new(&mut s.inner).poll(cx).is_ready());
    assert!(Pin::new(&mut r.wrapped).poll(cx).is_ready());
}

/// sea-streamer `consumer/future.rs`: `FutureExt::poll_unpin` on the stored
/// future.
#[test]
fn downstream_pattern_sea_streamer_poll_unpin() {
    use futures::FutureExt as _;
    let (tx, rx) = unbounded::<u32>();
    tx.send(9).unwrap();
    let mut fut: RecvFuture<'_, u32> = rx.recv_async();
    let cx = &mut Context::from_waker(futures::task::noop_waker_ref());
    assert_eq!(fut.poll_unpin(cx), Poll::Ready(Ok(9)));
}
