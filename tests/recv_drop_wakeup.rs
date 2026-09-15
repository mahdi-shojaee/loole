//! Receiver-drop wakeup tests: a `RecvFuture` dropped after a sender claimed
//! its registration but before it was re-polled (e.g. a losing `select!`
//! branch or an aborted task) must wake the next parked receiver, or the
//! message the sender left behind would be stranded while receivers sleep.
//! Conversely, a future that completed normally must not wake anyone on drop.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::Duration;

use futures::task::{waker, ArcWake};
use loole::bounded;

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
fn drop_woken_recv_future_wakes_next_waiter() {
    // Queue delivery: the sender pops f1's pending-recv registration, puts the
    // message in the queue, and wakes f1's (noop) waker. Dropping f1 without
    // re-polling it must wake f2, or the message is stranded while f2 sleeps.
    let (tx, rx) = bounded(1);

    let mut f1 = rx.recv_async();
    let noop = &mut Context::from_waker(futures::task::noop_waker_ref());
    assert!(Pin::new(&mut f1).poll(noop).is_pending());

    let (count, w) = counting_waker();
    let cx2 = &mut Context::from_waker(&w);
    let mut f2 = rx.recv_async();
    assert!(Pin::new(&mut f2).poll(cx2).is_pending());

    tx.send(42).unwrap(); // claims f1's registration, delivers to the queue
    assert_eq!(count.load(Ordering::SeqCst), 0);

    drop(f1); // dropped without re-polling: owes f2 a wake
    assert_eq!(count.load(Ordering::SeqCst), 1);

    // The woken f2 receives the message that was left behind.
    assert_eq!(Pin::new(&mut f2).poll(cx2), Poll::Ready(Ok(42)));
}

#[test]
fn drop_woken_recv_future_wakes_next_waiter_rendezvous() {
    // Rendezvous (cap-0): the sender pops f1's registration and leaves the
    // message as a gift in pending-sends. Dropping f1 without re-polling must
    // wake f2 so it can claim the gift.
    let (tx, rx) = bounded::<i32>(0);

    let mut f1 = rx.recv_async();
    let noop = &mut Context::from_waker(futures::task::noop_waker_ref());
    assert!(Pin::new(&mut f1).poll(noop).is_pending());

    let (count, w) = counting_waker();
    let cx2 = &mut Context::from_waker(&w);
    let mut f2 = rx.recv_async();
    assert!(Pin::new(&mut f2).poll(cx2).is_pending());

    tx.send(7).unwrap(); // claims f1's registration, leaves a Gift
    assert_eq!(count.load(Ordering::SeqCst), 0);

    drop(f1); // dropped without re-polling: owes f2 a wake
    assert_eq!(count.load(Ordering::SeqCst), 1);

    assert_eq!(Pin::new(&mut f2).poll(cx2), Poll::Ready(Ok(7)));
}

#[test]
fn completed_recv_future_drop_does_not_wake() {
    // The compensating wake exists for claimed-and-never-re-polled futures
    // only: a future that resolved normally must drop silently.
    let (tx, rx) = bounded(2);

    let mut f1 = rx.recv_async();
    let noop = &mut Context::from_waker(futures::task::noop_waker_ref());
    assert!(Pin::new(&mut f1).poll(noop).is_pending());

    let (count, w) = counting_waker();
    let cx2 = &mut Context::from_waker(&w);
    let mut f2 = rx.recv_async();
    assert!(Pin::new(&mut f2).poll(cx2).is_pending());

    tx.send(1).unwrap(); // claims f1's registration, delivers to the queue
                         // f1 re-polls after the wake and completes normally.
    assert_eq!(Pin::new(&mut f1).poll(noop), Poll::Ready(Ok(1)));

    drop(f1); // completed: no message was left behind, no wake owed
    assert_eq!(count.load(Ordering::SeqCst), 0);

    // f2's registration is intact: a new send wakes it.
    tx.send(2).unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(Pin::new(&mut f2).poll(cx2), Poll::Ready(Ok(2)));
}

#[test]
fn drop_woken_recv_future_wakes_parked_sync_receiver() {
    // End-to-end with a sync receiver: its Hook is notified (queue delivery)
    // and it picks the message up from the queue.
    let (tx, rx) = bounded(1);

    let mut f1 = rx.recv_async();
    let noop = &mut Context::from_waker(futures::task::noop_waker_ref());
    assert!(Pin::new(&mut f1).poll(noop).is_pending());

    let rx2 = rx.clone();
    let h = thread::spawn(move || rx2.recv_timeout(Duration::from_secs(5)));
    sync_sleep(100); // let the sync receiver park behind f1

    tx.send(42).unwrap(); // claims f1's registration, delivers to the queue
    drop(f1); // dropped without re-polling: owes the sync receiver a wake

    assert_eq!(h.join().unwrap(), Ok(42));
}
