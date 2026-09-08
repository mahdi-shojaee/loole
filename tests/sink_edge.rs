//! Remaining `Sink` implementation paths not covered by `tests/sink.rs`.

use futures::sink::Sink;
use futures::sink::SinkExt;
use loole::{bounded, unbounded, SendError};
use std::pin::Pin;
use std::task::{Context, Poll};

#[tokio::test]
async fn send_sink_sender_accessor() {
    // `SendSink::sender()` must return a handle to the same channel as the
    // original sender, and remain usable for direct sends.
    let (tx, rx) = bounded(1);
    let sink = tx.sink();

    assert!(sink.sender().same_channel(&tx));
    assert_eq!(sink.sender().capacity(), Some(1));

    // Sends via the sink and via the accessor interleave in FIFO order.
    let mut sink = sink.clone();
    sink.send(1).await.unwrap();
    // Free space first so the accessor's send does not park.
    assert_eq!(rx.recv_async().await, Ok(1));
    sink.sender().send_async(2).await.unwrap();
    assert_eq!(rx.recv_async().await, Ok(2));

    // A different channel's sink is not the same channel.
    let (tx2, _rx2) = unbounded::<i32>();
    assert!(!sink.sender().same_channel(&tx2));
}

#[tokio::test]
async fn send_sink_poll_close_with_pending_message_fails_after_receiver_drop() {
    // `poll_close` first flushes the pending message; if the receiver is
    // dropped while the message is still buffered in the sink, the flush (and
    // thus the close) must fail with `SendError`.
    let (tx, rx) = bounded::<i32>(1);
    let mut sink = tx.into_sink();

    let noop = futures::task::noop_waker();
    let cx = &mut Context::from_waker(&noop);
    Pin::new(&mut sink).start_send(7).unwrap();
    drop(rx);

    match Pin::new(&mut sink).poll_close(cx) {
        Poll::Ready(Err(SendError(7))) => {}
        other => panic!("expected Ready(Err(SendError(7))), got {other:?}"),
    }
}

#[tokio::test]
async fn send_sink_poll_flush_error_path() {
    // `poll_flush` on a sink with a pending message whose receiver was
    // dropped must return `Err(SendError)` carrying the message.
    let (tx, rx) = bounded::<i32>(1);
    let mut sink = tx.into_sink();

    let noop = futures::task::noop_waker();
    let cx = &mut Context::from_waker(&noop);
    Pin::new(&mut sink).start_send(5).unwrap();
    drop(rx);

    match Pin::new(&mut sink).poll_flush(cx) {
        Poll::Ready(Err(SendError(5))) => {}
        other => panic!("expected Ready(Err(SendError(5))), got {other:?}"),
    }
}

#[tokio::test]
async fn send_sink_close_with_pending_message_fails_after_receiver_drop() {
    // Same as the manual `poll_close` test above, but via the `SinkExt::close`
    // combinator. Note that when the flush fails, the `?` in `poll_close`
    // returns before `sender.close()` runs, so the channel is closed by the
    // receiver drop, not by the failed close.
    let (tx, rx) = bounded::<i32>(1);
    let mut sink = tx.into_sink();

    Pin::new(&mut sink).start_send(7).unwrap();
    drop(rx);

    let err = sink.close().await.unwrap_err();
    assert_eq!(err.into_inner(), 7);
}

#[tokio::test]
async fn send_sink_flush_blocks_until_space() {
    // The sink's flush awaits the pending message when the channel is full,
    // and completes once a receiver frees space.
    let (tx, rx) = bounded(1);
    let mut sink = tx.into_sink();

    sink.send(1).await.unwrap();

    let flush_fut = sink.send(2);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    // Not yet flushed: the channel is full with message 1.
    assert_eq!(rx.len(), 1);

    assert_eq!(rx.recv_async().await, Ok(1));
    flush_fut.await.unwrap();
    assert_eq!(rx.recv_async().await, Ok(2));
}

#[tokio::test]
async fn send_sink_send_error_payload_preserved() {
    let (tx, rx) = bounded(1);
    tx.send(1).unwrap();
    drop(rx);

    let mut sink = tx.sink();
    let err = sink.send(2).await.unwrap_err();
    assert_eq!(err.into_inner(), 2);
}

#[tokio::test]
async fn send_sink_on_rendezvous_channel() {
    // A sink on a rendezvous channel parks inside flush until a receiver is
    // ready, then hands the message off.
    let (tx, rx) = bounded(0);
    let mut sink = tx.into_sink();

    let send_task = tokio::spawn(async move {
        sink.send(42).await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(rx.recv_async().await, Ok(42));
    send_task.await.unwrap();
}

#[tokio::test]
async fn sender_into_sink_moves_ownership() {
    // `into_sink` consumes the sender; the sink keeps the channel open until
    // dropped (last sender handle).
    let (tx, rx) = unbounded::<i32>();
    let mut sink = tx.into_sink();
    sink.send(1).await.unwrap();
    // Only the sink holds a sender handle now; the channel stays open...
    assert!(!rx.is_closed());
    // ...and closes once the sink is dropped.
    drop(sink);
    assert!(rx.is_closed());
    assert_eq!(rx.recv_async().await, Ok(1));
    assert_eq!(rx.recv_async().await, Err(loole::RecvError::Disconnected));
}

#[tokio::test]
async fn cloned_sink_keeps_channel_open() {
    let (tx, rx) = unbounded::<i32>();
    let mut sink1 = tx.sink();
    let mut sink2 = sink1.clone();
    drop(tx);

    sink1.send(1).await.unwrap();
    sink2.send(2).await.unwrap();
    drop(sink1);
    // The clone keeps the channel open.
    assert!(!rx.is_closed());
    sink2.send(3).await.unwrap();
    drop(sink2);
    assert!(rx.is_closed());
    assert_eq!(rx.recv_async().await, Ok(1));
    assert_eq!(rx.recv_async().await, Ok(2));
    assert_eq!(rx.recv_async().await, Ok(3));
}

#[tokio::test]
async fn send_sink_debug_and_same_channel() {
    let (tx, _rx) = unbounded::<i32>();
    let s1 = tx.sink();
    let s2 = tx.sink();
    assert!(s1.same_channel(&s2));

    let (tx2, _rx2) = unbounded::<i32>();
    let s3 = tx2.sink();
    assert!(!s1.same_channel(&s3));

    // `Debug` is implemented for the sink.
    let _ = format!("{:?}", s1);
    let _ = format!("{:?}", tx);
}

#[test]
fn send_sink_convenience_methods_on_rendezvous_channel() {
    // On a rendezvous channel the sink reports `is_full() == true` and
    // `len() == 0` at all times.
    let (tx, _rx) = bounded::<()>(0);
    let sink = tx.sink();
    assert!(sink.is_full());
    assert!(sink.is_empty());
    assert_eq!(sink.len(), 0);
    assert_eq!(sink.capacity(), Some(0));
    assert!(!sink.is_closed());
}

#[test]
fn sink_from_sender_method() {
    // `Sender::sink()` (borrowing variant) does not consume the sender.
    let (tx, rx) = unbounded::<u8>();
    let sink = tx.sink();
    // The sender is still usable directly.
    tx.send(1).unwrap();
    assert_eq!(rx.recv(), Ok(1));
    // The sink's sender accessor is usable too.
    sink.sender().send(2).unwrap();
    assert_eq!(rx.recv(), Ok(2));
}
