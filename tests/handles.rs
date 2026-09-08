//! Tests for handle clone/drop semantics, convenience methods of the
//! future/sink/stream wrappers on rendezvous channels, and `Debug` impls.

use loole::{bounded, unbounded};

#[test]
fn dropping_one_of_several_cloned_senders_does_not_close() {
    let (tx, rx) = unbounded::<()>();
    let tx2 = tx.clone();
    let tx3 = tx2.clone();
    drop(tx);
    drop(tx2);
    // tx3 still holds the channel open.
    assert!(!rx.is_closed());
    tx3.send(()).unwrap();
    assert_eq!(rx.recv(), Ok(()));
    // The last sender drop closes.
    drop(tx3);
    assert!(rx.is_closed());
}

#[test]
fn dropping_one_of_several_cloned_receivers_does_not_close() {
    let (tx, rx) = unbounded::<()>();
    let rx2 = rx.clone();
    let rx3 = rx2.clone();
    drop(rx);
    drop(rx2);
    // rx3 still holds the channel open.
    assert!(!tx.is_closed());
    tx.send(()).unwrap();
    assert_eq!(rx3.recv(), Ok(()));
    // The last receiver drop closes.
    drop(rx3);
    assert!(tx.is_closed());
}

#[test]
fn send_future_keeps_channel_open_until_dropped() {
    // `send_async` clones the sender into the future; a pending future keeps
    // the channel open even after the original sender is dropped.
    let (tx, rx) = bounded::<u8>(1);
    let fut = tx.send_async(1);
    drop(tx);
    assert!(!rx.is_closed());
    drop(fut);
    // The future was the last sender handle.
    assert!(rx.is_closed());
}

#[test]
fn recv_future_keeps_channel_open_until_dropped() {
    // `recv_async` clones the receiver into the future; a pending future
    // keeps the channel open even after the original receiver is dropped.
    let (tx, rx) = unbounded::<u8>();
    let fut = rx.recv_async();
    drop(rx);
    assert!(!tx.is_closed());
    drop(fut);
    // The future was the last receiver handle.
    assert!(tx.is_closed());
}

#[test]
fn recv_stream_keeps_channel_open_until_dropped() {
    let (tx, rx) = unbounded::<u8>();
    let stream = rx.stream();
    drop(rx);
    assert!(!tx.is_closed());
    drop(stream);
    assert!(tx.is_closed());
}

#[test]
fn rendezvous_channel_convenience_methods() {
    // A rendezvous channel is always empty AND always full, and this must be
    // visible through every wrapper type: `SendFuture`, `SendSink`,
    // `RecvFuture`, and `RecvStream`.
    let (tx, rx) = bounded::<()>(0);

    assert_eq!(tx.capacity(), Some(0));
    assert!(tx.is_empty());
    assert!(tx.is_full());
    assert_eq!(tx.len(), 0);
    assert!(!tx.is_closed());

    let send_fut = tx.send_async(());
    assert_eq!(send_fut.capacity(), Some(0));
    assert!(send_fut.is_empty());
    assert!(send_fut.is_full());
    assert_eq!(send_fut.len(), 0);
    drop(send_fut);

    let sink = tx.sink();
    assert_eq!(sink.capacity(), Some(0));
    assert!(sink.is_empty());
    assert!(sink.is_full());
    assert_eq!(sink.len(), 0);
    drop(sink);

    assert_eq!(rx.capacity(), Some(0));
    assert!(rx.is_empty());
    assert!(rx.is_full());
    assert_eq!(rx.len(), 0);

    let recv_fut = rx.recv_async();
    assert_eq!(recv_fut.capacity(), Some(0));
    assert!(recv_fut.is_empty());
    assert!(recv_fut.is_full());
    assert_eq!(recv_fut.len(), 0);
    drop(recv_fut);

    let stream = rx.stream();
    assert_eq!(stream.capacity(), Some(0));
    assert!(stream.is_empty());
    assert!(stream.is_full());
    assert_eq!(stream.len(), 0);
    drop(stream);

    // `same_channel` for the wrappers.
    assert!(tx.same_channel(&tx.clone()));
    assert!(rx.same_channel(&rx.clone()));
}

#[test]
fn unbounded_channel_convenience_methods() {
    let (tx, rx) = unbounded::<u8>();

    assert_eq!(tx.capacity(), None);
    assert!(tx.is_empty());
    assert!(!tx.is_full());
    assert_eq!(tx.len(), 0);

    let send_fut = tx.send_async(1);
    assert_eq!(send_fut.capacity(), None);
    assert!(send_fut.is_empty());
    assert!(!send_fut.is_full());
    drop(send_fut);

    let recv_fut = rx.recv_async();
    assert_eq!(recv_fut.capacity(), None);
    assert!(recv_fut.is_empty());
    assert!(!recv_fut.is_full());
    drop(recv_fut);

    tx.send(1).unwrap();
    assert!(!tx.is_empty());
    // An unbounded channel is never full.
    assert!(!tx.is_full());
    assert_eq!(tx.len(), 1);
    assert_eq!(rx.len(), 1);
    assert_eq!(rx.recv(), Ok(1));
}

#[test]
fn debug_formatting_of_handles_and_wrappers() {
    let (tx, rx) = bounded::<()>(1);

    // All handles and wrappers implement `Debug` regardless of the message type.
    let _ = format!("{:?}", tx);
    let _ = format!("{:?}", rx);

    let send_fut = tx.send_async(());
    let _ = format!("{:?}", send_fut);
    drop(send_fut);

    let sink = tx.sink();
    let _ = format!("{:?}", sink);
    drop(sink);

    let recv_fut = rx.recv_async();
    let _ = format!("{:?}", recv_fut);
    drop(recv_fut);

    let stream = rx.stream();
    let _ = format!("{:?}", stream);
}

#[test]
fn sink_and_stream_same_channel_semantics() {
    let (tx1, _rx1) = unbounded::<u8>();
    let (tx2, _rx2) = unbounded::<u8>();

    let sink1 = tx1.sink();
    let sink1_clone = sink1.clone();
    let sink2 = tx2.sink();
    assert!(sink1.same_channel(&sink1_clone));
    assert!(!sink1.same_channel(&sink2));

    let (_tx1, rx1) = unbounded::<u8>();
    let (_tx2, rx2) = unbounded::<u8>();
    let stream1 = rx1.stream();
    let stream1_clone = stream1.clone();
    let stream2 = rx2.stream();
    assert!(stream1.same_channel(&stream1_clone));
    assert!(!stream1.same_channel(&stream2));
}

#[test]
fn cloned_sender_can_be_sent_across_threads() {
    // Clones of both halves remain usable when moved to another thread, and
    // the channel stays open as long as any handle exists.
    let (tx, rx) = unbounded::<u8>();
    let tx_clone = tx.clone();
    let h = std::thread::spawn(move || {
        tx_clone.send(1).unwrap();
        // Return the clone so it stays alive after the thread body ends.
        tx_clone
    });
    drop(tx);
    assert_eq!(rx.recv(), Ok(1));
    // The thread's clone keeps the channel open...
    let tx_returned = h.join().unwrap();
    assert!(!rx.is_closed());
    tx_returned.send(2).unwrap();
    assert_eq!(rx.recv(), Ok(2));
    // ...until it is dropped.
    drop(tx_returned);
    assert!(rx.is_closed());
}
