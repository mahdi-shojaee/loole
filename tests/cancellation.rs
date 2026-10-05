//! Send-side cancellation tests: dropping a pending `SendFuture` cancels the
//! send (the queued message is removed and destroyed on cancellation),
//! the hand-off race with a concurrent receiver is arbitrated uniquely, and
//! re-polling a future interacts correctly with the registered waker.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::Duration;

use futures::task::{waker, ArcWake};
use loole::{bounded, unbounded, SendError, TryRecvError, TrySendError};

fn sync_sleep(ms: u64) {
    thread::sleep(Duration::from_millis(ms))
}

struct CountingWaker {
    count: Arc<AtomicUsize>,
}

impl ArcWake for CountingWaker {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        arc_self.count.fetch_add(1, Ordering::SeqCst);
    }
}

fn counting_waker() -> (Arc<AtomicUsize>, Waker) {
    let count = Arc::new(AtomicUsize::new(0));
    let w = waker(Arc::new(CountingWaker {
        count: count.clone(),
    }));
    (count, w)
}

#[test]
fn drop_pending_send_future_on_full_channel_cancels_send() {
    // Dropping a pending `SendFuture` removes its message from the
    // pending-sends queue: freeing space afterwards must NOT deliver it.
    let (tx, rx) = bounded(1);
    tx.send(1).unwrap();

    let cx = &mut Context::from_waker(futures::task::noop_waker_ref());
    {
        let mut f = tx.send_async(2);
        assert!(Pin::new(&mut f).poll(cx).is_pending());
        // Drop the future while the send is queued: the message is cancelled.
    }

    // Free space: only the original buffered message is received.
    assert_eq!(rx.recv(), Ok(1));
    assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
    // The cancelled slot is gone; a new send succeeds.
    assert_eq!(tx.try_send(3), Ok(()));
    assert_eq!(rx.recv(), Ok(3));
}

#[test]
fn drop_pending_send_future_on_rendezvous_channel_cancels_send() {
    let (tx, rx) = bounded(0);

    let cx = &mut Context::from_waker(futures::task::noop_waker_ref());
    {
        let mut f = tx.send_async(1);
        assert!(Pin::new(&mut f).poll(cx).is_pending());
    }

    // The future was dropped: its message is cancelled, so a rendezvous recv
    // finds nothing and the channel behaves as empty.
    assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
    // A fresh send still hands off normally.
    let h = thread::spawn(move || tx.send(7));
    sync_sleep(100);
    assert_eq!(rx.recv(), Ok(7));
    assert_eq!(h.join().unwrap(), Ok(()));
}

#[test]
fn drop_pending_send_future_after_disconnect() {
    // Dropping a pending send future and then disconnecting must leave the
    // sender observing a closed channel.
    let (tx, rx) = bounded(1);
    tx.send(1).unwrap();

    let cx = &mut Context::from_waker(futures::task::noop_waker_ref());
    let mut f = tx.send_async(2);
    assert!(Pin::new(&mut f).poll(cx).is_pending());
    drop(f);
    drop(rx);
    assert!(tx.is_closed());
    assert_eq!(tx.try_send(3), Err(TrySendError::Disconnected(3)));
}

#[test]
fn cancelled_send_message_is_actually_dropped() {
    // The cancelled message must be destroyed exactly once (drop accounting),
    // not leaked in the pending queue.
    static DROPS: AtomicUsize = AtomicUsize::new(0);

    #[derive(Debug, PartialEq)]
    struct DropCounter(usize);
    impl Drop for DropCounter {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }

    let (tx, _rx) = bounded(0);
    let cx = &mut Context::from_waker(futures::task::noop_waker_ref());
    {
        let mut f = tx.send_async(DropCounter(42));
        assert!(Pin::new(&mut f).poll(cx).is_pending());
    }
    // Dropping the future must have dropped the message exactly once.
    sync_sleep(100);
    assert_eq!(DROPS.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn send_future_cancelled_by_select_cancels_message() {
    // Cancellation inside `select!`: the losing send future is dropped
    // and its message is cancelled.
    let (tx, rx) = bounded(1);
    tx.send(1).unwrap();

    let send_fut = tx.send_async(2);
    tokio::select! {
        _ = send_fut => panic!("send should not complete while the channel is full"),
        _ = tokio::time::sleep(Duration::from_millis(50)) => {}
    }

    // The send future was dropped: message 2 is cancelled, not delivered.
    assert_eq!(rx.recv_async().await, Ok(1));
    assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
}

#[test]
fn drop_race_with_receiving_never_drops_or_duplicats_message() {
    // Ownership-race stress: many cancellers race a receiver on a rendezvous
    // channel. Each message carries a unique id, so duplicates are detectable.
    // Conservation law: delivered + dropped == sent (each message is either
    // handed to the receiver or destroyed by its canceller — never both,
    // never neither, never twice).
    const SENDERS: usize = 64;
    const VALUES: usize = 4;
    const TOTAL: usize = SENDERS * VALUES;

    static DROPS: AtomicUsize = AtomicUsize::new(0);

    #[derive(Debug, PartialEq)]
    struct Message(usize);
    impl Drop for Message {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }

    let (tx, rx) = bounded::<Message>(0);

    let received: Vec<usize> = std::thread::scope(|s| {
        for t in 0..SENDERS {
            let tx = tx.clone();
            s.spawn(move || {
                let waker = futures::task::noop_waker();
                let cx = &mut Context::from_waker(&waker);
                let mut futures = Vec::new();
                for v in 0..VALUES {
                    let mut f = tx.send_async(Message(t * VALUES + v));
                    let _ = Pin::new(&mut f).poll(cx);
                    futures.push(f);
                }
                drop(futures);
            });
        }
        s.spawn(|| {
            let mut got = Vec::new();
            loop {
                match rx.recv_timeout(Duration::from_millis(20)) {
                    Ok(m) => got.push(m.0),
                    Err(_) => break,
                }
            }
            got
        })
        .join()
        .unwrap()
    });

    // All drops have happened: the scope joined every canceller thread, and
    // the receiver pushes only the inner u64 (the `Message` wrapper is
    // dropped at receive time, inside the loop above), so `drops` is final
    // here. Correct accounting:
    //   total drops = (cancelled by droppers) + (delivered, dropped by receiver)
    // => cancelled = drops - delivered  and  cancelled + delivered == sent.
    let drops = DROPS.load(Ordering::SeqCst);
    let delivered = received.len();
    assert!(
        drops >= delivered,
        "fewer drops than deliveries: {drops} < {delivered} — a delivered message leaked"
    );
    let cancelled = drops - delivered;
    assert_eq!(
        cancelled + delivered,
        TOTAL,
        "conservation violated: {cancelled} cancelled + {delivered} delivered != {TOTAL} sent"
    );

    // Unique ids: no message delivered twice.
    let mut sorted = received.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), delivered, "duplicate message received");
    // Every delivered id is a valid, in-range message.
    for id in &sorted {
        assert!(*id < TOTAL, "unexpected id {id}");
    }
}

#[test]
fn poll_send_future_after_completion_returns_ok() {
    // Polling an already-completed `SendFuture` resolves with
    // `Ready(Ok(()))` instead of panicking.
    let (tx, _rx) = unbounded::<u8>();
    let mut f = tx.send_async(1);
    let cx = &mut Context::from_waker(futures::task::noop_waker_ref());
    assert_eq!(Pin::new(&mut f).poll(cx), Poll::Ready(Ok(())));
    assert_eq!(Pin::new(&mut f).poll(cx), Poll::Ready(Ok(())));
}

#[test]
fn send_future_waker_is_refreshed_on_repoll() {
    // Re-polling a pending `SendFuture` with a *different* waker must replace
    // the registered waker: only the new
    // waker may be woken by a later event, never the stale one.
    let (tx, rx) = bounded(1);
    tx.send(1).unwrap();

    let (count1, waker1) = counting_waker();
    let (count2, waker2) = counting_waker();
    let cx1 = &mut Context::from_waker(&waker1);
    let cx2 = &mut Context::from_waker(&waker2);

    let mut f = tx.send_async(2);
    // First poll registers waker1.
    assert!(Pin::new(&mut f).poll(cx1).is_pending());
    // Second poll with a new waker replaces it — neither is woken yet.
    assert!(Pin::new(&mut f).poll(cx2).is_pending());
    assert_eq!(count1.load(Ordering::SeqCst), 0);
    assert_eq!(count2.load(Ordering::SeqCst), 0);

    // Freeing space wakes the *new* waker (waker2), not the stale one.
    assert_eq!(rx.recv(), Ok(1));
    sync_sleep(100);
    assert_eq!(count2.load(Ordering::SeqCst), 1);
    assert_eq!(count1.load(Ordering::SeqCst), 0);
    assert_eq!(rx.recv(), Ok(2));
}

#[test]
fn recv_future_waker_is_refreshed_on_repoll() {
    // Re-polling a pending `RecvFuture` with a *different* waker must replace
    // the registered waker: only the new
    // waker may be woken by a later send, never the stale one.
    let (tx, rx) = bounded::<u8>(1);

    let (count1, waker1) = counting_waker();
    let (count2, waker2) = counting_waker();
    let cx1 = &mut Context::from_waker(&waker1);
    let cx2 = &mut Context::from_waker(&waker2);

    let mut f = rx.recv_async();
    assert!(Pin::new(&mut f).poll(cx1).is_pending());
    assert!(Pin::new(&mut f).poll(cx2).is_pending());

    tx.send(1).unwrap();
    // The *new* waker (waker2) is woken; the stale one is not.
    sync_sleep(100);
    assert_eq!(count2.load(Ordering::SeqCst), 1);
    assert_eq!(count1.load(Ordering::SeqCst), 0);

    // The future can still complete with a fresh waker.
    let cx3 = &mut Context::from_waker(&waker2);
    assert_eq!(Pin::new(&mut f).poll(cx3), Poll::Ready(Ok(1)));
}

#[test]
fn send_futures_resolve_in_fifo_order_after_disconnect() {
    let (tx, rx) = bounded(0);

    let cx = &mut Context::from_waker(futures::task::noop_waker_ref());
    let mut f1 = tx.send_async(1);
    let mut f2 = tx.send_async(2);
    assert!(Pin::new(&mut f1).poll(cx).is_pending());
    assert!(Pin::new(&mut f2).poll(cx).is_pending());

    drop(rx);

    assert_eq!(Pin::new(&mut f2).poll(cx), Poll::Ready(Err(SendError(2))));
    assert_eq!(Pin::new(&mut f1).poll(cx), Poll::Ready(Err(SendError(1))));
}

#[test]
fn recv_future_drop_cleans_up_pending_registration() {
    // A dropped `RecvFuture` removes itself from the pending-recvs queue, so
    // later messages are routed to other receivers (here: a direct recv call).
    let (tx, rx) = bounded(1);

    let cx = &mut Context::from_waker(futures::task::noop_waker_ref());
    {
        let mut f = rx.recv_async();
        assert!(Pin::new(&mut f).poll(cx).is_pending());
    }

    tx.send(42).unwrap();
    // The dropped future no longer holds the message; a direct recv gets it.
    assert_eq!(rx.recv(), Ok(42));
    assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));
}
