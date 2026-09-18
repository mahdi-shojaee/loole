use std::thread::{self, scope};
use std::time::Duration;

use loole::{bounded, unbounded, RecvError, RecvTimeoutError, SendError, SendTimeoutError, TryRecvError, TrySendError};

fn ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

fn sync_sleep(ms: u64) {
    thread::sleep(Duration::from_millis(ms))
}

#[test]
fn close_is_idempotent() {
    let (tx, rx) = bounded::<()>(1);
    assert!(!tx.is_closed());
    assert!(!rx.is_closed());
    assert!(tx.close());
    assert!(tx.is_closed());
    assert!(rx.is_closed());
    // Closing again reports that it did not actively close the channel,
    // whether from the sender or the receiver side.
    assert!(!tx.close());
    assert!(!rx.close());
}

#[test]
fn close_by_receiver_is_idempotent() {
    let (tx, rx) = bounded::<()>(1);
    assert!(rx.close());
    assert!(tx.is_closed());
    assert!(!rx.close());
    assert!(!tx.close());
}

#[test]
fn close_wakes_blocked_sync_sender() {
    let (tx, rx) = bounded(1);
    tx.send(1).unwrap();
    scope(|scope| {
        let h = scope.spawn(move || {
            // This blocks because the channel is full.
            tx.send(2)
        });
        sync_sleep(100);
        assert!(rx.close());
        // The blocked sender is woken with `SendError`, and its message is
        // returned as the error payload.
        assert_eq!(h.join().unwrap(), Err(SendError(2)));
    });
}

#[test]
fn close_wakes_blocked_sync_sender_on_rendezvous_channel() {
    let (tx, rx) = bounded::<u8>(0);
    scope(|scope| {
        let h = scope.spawn(move || tx.send(7));
        sync_sleep(100);
        assert!(rx.close());
        assert_eq!(h.join().unwrap(), Err(SendError(7)));
    });
}

#[test]
fn close_wakes_blocked_sync_receiver() {
    let (tx, rx) = bounded::<()>(1);
    scope(|scope| {
        let h = scope.spawn(move || rx.recv());
        sync_sleep(100);
        assert!(tx.close());
        assert_eq!(h.join().unwrap(), Err(RecvError::Disconnected));
    });
}

#[test]
fn close_wakes_blocked_sync_sender_of_timed_send() {
    let (tx, rx) = bounded(1);
    tx.send(1).unwrap();
    scope(|scope| {
        let h = scope.spawn(move || tx.send_timeout(2, ms(10_000)));
        sync_sleep(100);
        assert!(rx.close());
        assert_eq!(h.join().unwrap(), Err(SendTimeoutError::Disconnected(2)));
    });
}

#[test]
fn close_wakes_blocked_sync_receiver_of_timed_recv() {
    let (tx, rx) = bounded::<()>(1);
    scope(|scope| {
        let h = scope.spawn(move || rx.recv_timeout(ms(10_000)));
        sync_sleep(100);
        assert!(tx.close());
        assert_eq!(h.join().unwrap(), Err(RecvTimeoutError::Disconnected));
    });
}

#[test]
fn close_wakes_pending_recv_async() {
    let (tx, rx) = bounded::<()>(1);
    let rx_clone = rx.clone();
    scope(|scope| {
        let h = scope.spawn(move || {
            async_std::task::block_on(async { rx_clone.recv_async().await })
        });
        sync_sleep(100);
        assert!(tx.close());
        assert_eq!(h.join().unwrap(), Err(RecvError::Disconnected));
    });
}

#[test]
fn close_wakes_pending_send_async() {
    let (tx, rx) = bounded::<u8>(0);
    let tx_clone = tx.clone();
    scope(|scope| {
        let h = scope.spawn(move || {
            async_std::task::block_on(async { tx_clone.send_async(9).await })
        });
        sync_sleep(100);
        assert!(rx.close());
        assert_eq!(h.join().unwrap(), Err(SendError(9)));
    });
}

#[test]
fn close_keeps_pending_messages_receivable() {
    // A manual close keeps already-queued messages receivable.
    let (tx, rx) = unbounded();
    tx.send(1).unwrap();
    tx.send(2).unwrap();
    assert!(tx.close());
    assert_eq!(rx.recv(), Ok(1));
    assert_eq!(rx.recv(), Ok(2));
    assert_eq!(rx.recv(), Err(RecvError::Disconnected));
}

#[test]
fn send_after_manual_close_fails() {
    let (tx, rx) = bounded::<()>(1);
    assert!(rx.close());
    assert_eq!(tx.send(()), Err(SendError(())));
    assert_eq!(tx.try_send(()), Err(TrySendError::Disconnected(())));
}

#[test]
fn recv_after_manual_close_by_receiver_fails_immediately() {
    let (tx, rx) = bounded::<()>(1);
    assert!(rx.close());
    assert_eq!(rx.recv(), Err(RecvError::Disconnected));
    assert_eq!(rx.try_recv(), Err(TryRecvError::Disconnected));
    // The sender sees the channel as closed too.
    assert!(tx.is_closed());
}

#[tokio::test]
async fn close_wakes_pending_recv_future() {
    let (tx, rx) = bounded::<()>(1);
    let recv_task = tokio::spawn(rx.into_recv_async());
    tokio::time::sleep(ms(100)).await;
    assert!(tx.close());
    assert_eq!(recv_task.await.unwrap(), Err(RecvError::Disconnected));
}

#[tokio::test]
async fn close_wakes_pending_send_future() {
    let (tx, rx) = bounded(1);
    tx.send_async(1).await.unwrap();
    let send_task = tokio::spawn(tx.into_send_async(2));
    tokio::time::sleep(ms(100)).await;
    assert!(rx.close());
    assert_eq!(send_task.await.unwrap(), Err(SendError(2)));
}

#[tokio::test]
async fn send_sink_after_close_fails() {
    use futures::SinkExt;

    let (tx, rx) = unbounded();
    let mut sink = tx.into_sink();
    assert!(rx.close());
    // Sending through the sink after a manual close must fail with `SendError`.
    assert!(matches!(sink.send(1).await, Err(SendError(1))));
}
