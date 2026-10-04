//! Tests for `Receiver::drain` on rendezvous channels and `Drain`'s
//! `ExactSizeIterator` implementation.

use std::thread::{self, scope};
use std::time::Duration;

use loole::{bounded, unbounded};

fn sync_sleep(ms: u64) {
    thread::sleep(Duration::from_millis(ms))
}

#[test]
fn drain_on_rendezvous_channel_pulls_nothing() {
    // `drain` takes the queued items and pulls up to `cap` pending sends into
    // the queue. A rendezvous channel has no queue and `cap == 0`, so drain
    // must take nothing and leave the pending send blocked: the message must
    // not be pulled into the queue and must still be delivered by a regular
    // recv. Detection is via `len` (a pulled message would sit in the queue
    // after drain took its snapshot) — no timing assertions, so a delayed
    // thread start under CI load cannot false-fail the test.
    let (tx, rx) = bounded(0);
    scope(|scope| {
        let h = scope.spawn(move || {
            assert_eq!(tx.send(1), Ok(()));
        });
        // Give the sender time to park so drain actually faces a pending
        // send (coverage only: the assertions below hold either way).
        sync_sleep(100);
        let drained = rx.drain().collect::<Vec<_>>();
        assert!(drained.is_empty());
        // A rendezvous channel has no buffer: nothing may be pulled in.
        assert_eq!(rx.len(), 0);
        // The message was not stolen from the blocked sender; it is still
        // delivered by a regular recv, which also unblocks the sender.
        assert_eq!(rx.recv(), Ok(1));
        h.join().unwrap();
    });
}

#[test]
fn drain_wakes_pulled_pending_senders() {
    // When drain pulls pending sends into the (now empty) queue, it must wake
    // the blocked senders so their `send` calls return. The pulled messages'
    // relative order is the arrival order in the pending-sends queue.
    let (tx, rx) = bounded(2);
    let tx1 = tx.clone();
    let tx2 = tx.clone();
    scope(|scope| {
        scope.spawn(move || {
            sync_sleep(100);
            assert_eq!(tx1.send(3), Ok(()));
        });
        scope.spawn(move || {
            sync_sleep(100);
            assert_eq!(tx2.send(4), Ok(()));
        });
        // Fill the queue [1, 2] first so the cloned senders block.
        assert_eq!(tx.send(1), Ok(()));
        assert_eq!(tx.send(2), Ok(()));
        sync_sleep(1000);
        assert_eq!(rx.len(), 2);
        // Queue is [1, 2] and sends 3, 4 are pending. Drain takes [1, 2]
        // and pulls 3 and 4 into the queue (cap is 2), waking their senders.
        let drained = rx.drain().collect::<Vec<_>>();
        assert_eq!(drained, [1, 2]);
        // The pulled messages are already queued: len is 2 before any recv.
        assert_eq!(rx.len(), 2);
        let pulled = [rx.recv().unwrap(), rx.recv().unwrap()];
        assert!(
            pulled == [3, 4] || pulled == [4, 3],
            "unexpected pulled messages: {pulled:?}"
        );
    });
}

#[test]
fn drain_is_exact_size_iterator() {
    // `Drain` implements `ExactSizeIterator`: `len` and `size_hint` track the
    // number of remaining items.
    let (tx, rx) = unbounded();
    for i in 0..5 {
        tx.send(i).unwrap();
    }

    let mut drain = rx.drain();
    assert_eq!(drain.len(), 5);
    // `size_hint` is consistent with `ExactSizeIterator`: `(len, Some(len))`.
    assert_eq!(drain.size_hint(), (5, Some(5)));
    assert_eq!(drain.next(), Some(0));
    assert_eq!(drain.len(), 4);
    assert_eq!(drain.size_hint(), (4, Some(4)));
    let inner: Vec<_> = drain.by_ref().take(2).collect();
    assert_eq!(inner, [1, 2]);
    assert_eq!(drain.len(), 2);
    let rest: Vec<_> = drain.collect();
    assert_eq!(rest, [3, 4]);

    // The channel is empty after the drain, and new sends are still possible.
    tx.send(9).unwrap();
    assert_eq!(rx.recv(), Ok(9));
}

#[test]
fn drain_empty_channel_yields_no_items() {
    let (_tx, rx) = unbounded::<u8>();
    let drain = rx.drain();
    assert_eq!(drain.len(), 0);
    assert_eq!(drain.collect::<Vec<_>>(), []);
}

#[test]
fn drain_debug_formatting() {
    let (tx, rx) = unbounded();
    tx.send(1).unwrap();
    let drain = rx.drain();
    // `Drain` implements `Debug` without inspecting the message type.
    let _ = format!("{:?}", drain);
}

#[test]
fn drain_on_unbounded_channel_does_not_block() {
    // Unlike `try_iter`, `drain` snapshots the queue: messages sent after the
    // drain call are not included.
    let (tx, rx) = unbounded();
    tx.send(1).unwrap();
    tx.send(2).unwrap();
    let drained: Vec<_> = rx.drain().collect();
    assert_eq!(drained, [1, 2]);
    tx.send(3).unwrap();
    assert_eq!(rx.try_recv(), Ok(3));
}
