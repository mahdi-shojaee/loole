//! Tests for the parked async send's consumption-confirmation paths: the
//! lock-free consumed-park watermark fast path and its lock-based fallback
//! (close, spurious wake, cancellation).

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::task::noop_waker_ref;
use futures::StreamExt;
use loole::{bounded, SendError};

fn ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

/// A parked send whose message was consumed resolves `Ok` (exercising the
/// lock-free watermark confirm), and the message stays in the channel.
#[test]
fn parked_send_resolves_ok_when_consumed() {
    let (tx, rx) = bounded(1);
    tx.send(1).unwrap();

    let mut fut = tx.send_async(2);
    let mut cx = Context::from_waker(noop_waker_ref());
    assert!(Pin::new(&mut fut).poll(&mut cx).is_pending());

    // Consuming one message dequeues the parked send and publishes its id.
    assert_eq!(rx.try_recv(), Ok(1));

    // The parked future must resolve Ok, with the message delivered.
    assert_eq!(Pin::new(&mut fut).poll(&mut cx), Poll::Ready(Ok(())));
    assert_eq!(rx.try_recv(), Ok(2));
    assert!(rx.try_recv().is_err());
}

/// A parked send woken by close (no consumption) resolves `Err` with the
/// message — the watermark has not advanced, so the lock fallback runs.
#[test]
fn parked_send_err_on_close() {
    let (tx, rx) = bounded(1);
    tx.send(1).unwrap();

    let mut fut = tx.send_async(2);
    let mut cx = Context::from_waker(noop_waker_ref());
    assert!(Pin::new(&mut fut).poll(&mut cx).is_pending());

    drop(rx);

    assert_eq!(
        Pin::new(&mut fut).poll(&mut cx),
        Poll::Ready(Err(SendError(2)))
    );
}

/// After one consumption, a *newly* parked send must not be confirmed by the
/// stale watermark when the channel closes without consuming it.
#[test]
fn consumed_watermark_does_not_confirm_later_parked_send_on_close() {
    let (tx, rx) = bounded(1);
    tx.send(1).unwrap();

    let mut fut1 = tx.send_async(2);
    let mut cx = Context::from_waker(noop_waker_ref());
    assert!(Pin::new(&mut fut1).poll(&mut cx).is_pending());
    assert_eq!(rx.try_recv(), Ok(1));
    assert_eq!(Pin::new(&mut fut1).poll(&mut cx), Poll::Ready(Ok(())));
    // The queue now holds the accepted message 2 (len == cap).

    // Park another send; the watermark already advanced once, so it must
    // not match this new (higher) id. Closing must report Err, never a
    // false Ok.
    let mut fut2 = tx.send_async(3);
    assert!(Pin::new(&mut fut2).poll(&mut cx).is_pending());
    drop(rx);
    assert_eq!(
        Pin::new(&mut fut2).poll(&mut cx),
        Poll::Ready(Err(SendError(3)))
    );
}

/// A re-poll without any consumption (spurious wake) stays pending and keeps
/// the waker registration intact; a later consumption still resolves it.
#[test]
fn spurious_wake_keeps_send_pending() {
    let (tx, rx) = bounded(1);
    tx.send(1).unwrap();

    let mut fut = tx.send_async(2);
    let mut cx = Context::from_waker(noop_waker_ref());
    assert!(Pin::new(&mut fut).poll(&mut cx).is_pending());
    // Spurious re-poll: nothing consumed, must stay pending.
    assert!(Pin::new(&mut fut).poll(&mut cx).is_pending());

    assert_eq!(rx.try_recv(), Ok(1));
    assert_eq!(Pin::new(&mut fut).poll(&mut cx), Poll::Ready(Ok(())));
    assert_eq!(rx.try_recv(), Ok(2));
}

/// Multiple parked senders are consumed FIFO; after k consumptions exactly the
/// first k futures resolve Ok (watermark is monotonic in consumption order)
/// and the messages are delivered in park order.
#[test]
fn mpsc_parked_sends_fifo_consumption() {
    let (tx, rx) = bounded(0);
    let mut cx = Context::from_waker(noop_waker_ref());

    let mut futs: Vec<_> = (1..=4).map(|i| tx.send_async(i)).collect();
    for f in futs.iter_mut() {
        assert!(Pin::new(f).poll(&mut cx).is_pending());
    }

    for consumed in 1..=4 {
        assert_eq!(rx.try_recv(), Ok(consumed));
        for (i, f) in futs.iter_mut().enumerate() {
            let expected = if i < consumed {
                Poll::Ready(Ok(()))
            } else {
                Poll::Pending
            };
            assert_eq!(
                Pin::new(f).poll(&mut cx),
                expected,
                "future {} after consuming {} messages",
                i,
                consumed
            );
        }
    }
}

/// Cancelling a parked future (drop) removes its entry without consuming it;
/// the remaining senders still resolve Ok and the cancelled message is never
/// delivered, even though the watermark advances past its id.
#[test]
fn cancel_parked_send_keeps_channel_correct() {
    let (tx, rx) = bounded(0);
    let mut cx = Context::from_waker(noop_waker_ref());

    let mut f1 = tx.send_async(1);
    let mut f2 = tx.send_async(2);
    let mut f3 = tx.send_async(3);
    assert!(Pin::new(&mut f1).poll(&mut cx).is_pending());
    assert!(Pin::new(&mut f2).poll(&mut cx).is_pending());
    assert!(Pin::new(&mut f3).poll(&mut cx).is_pending());

    // Cancel the middle send: entry removed, message 2 must never appear.
    drop(f2);

    assert_eq!(rx.try_recv(), Ok(1));
    assert_eq!(Pin::new(&mut f1).poll(&mut cx), Poll::Ready(Ok(())));
    assert_eq!(rx.try_recv(), Ok(3));
    assert_eq!(Pin::new(&mut f3).poll(&mut cx), Poll::Ready(Ok(())));
    assert!(rx.try_recv().is_err());
}

/// `drain` consumes parked sends: it publishes their ids so the woken async
/// senders resolve Ok, the drained messages are returned, and the parked
/// messages moved into the channel remain receivable.
#[tokio::test]
async fn drain_resolves_parked_async_sends() {
    let (tx, rx) = bounded(1);
    tx.send(1).unwrap();

    let send_task = tokio::spawn(async move { tx.send_async(2).await });
    tokio::time::sleep(ms(50)).await; // let it park on the full queue

    let drained: Vec<i32> = rx.drain().collect();
    assert_eq!(drained, vec![1]);
    // The parked message was accepted into the channel by the drain.
    assert_eq!(send_task.await.unwrap(), Ok(()));
    assert_eq!(rx.try_recv(), Ok(2));
    assert!(rx.try_recv().is_err());
}

/// A stream receiver on a full channel drives the parked-sender handoff: the
/// pending message is delivered through the queue and the sender resolves.
#[tokio::test]
async fn stream_on_full_channel_with_parked_send() {
    let (tx, rx) = bounded(1);
    tx.send(1).unwrap();

    let send_task = tokio::spawn(async move { tx.send_async(2).await });
    tokio::time::sleep(ms(50)).await; // let it park

    let mut stream = rx.stream();
    assert_eq!(stream.next().await, Some(1));
    assert_eq!(send_task.await.unwrap(), Ok(()));
    assert_eq!(stream.next().await, Some(2));
    assert_eq!(stream.next().await, None);
}

/// Superseding a parked sink item removes the old entry without publishing
/// its id; the superseding send still completes and delivers exactly once.
#[test]
fn sink_supersede_parked_send() {
    use futures::sink::Sink;

    let (tx, rx) = bounded(1);
    tx.send(0).unwrap();

    let mut sink = std::pin::pin!(tx.sink());
    let mut cx = Context::from_waker(noop_waker_ref());

    sink.as_mut().start_send(1).unwrap();
    assert!(sink.as_mut().poll_flush(&mut cx).is_pending());

    // Supersede: item 1's parked entry is removed, item 2 parks fresh.
    sink.as_mut().start_send(2).unwrap();
    assert!(sink.as_mut().poll_flush(&mut cx).is_pending());

    assert_eq!(rx.try_recv(), Ok(0));
    assert_eq!(sink.as_mut().poll_flush(&mut cx), Poll::Ready(Ok(())));
    assert_eq!(rx.try_recv(), Ok(2));
    assert!(rx.try_recv().is_err());
}

/// Rendezvous async ping-pong: every message parks the sender and consumes
/// the parked entry, advancing the watermark each time.
#[tokio::test]
async fn cap0_async_pingpong_watermark_per_message() {
    let (tx, rx) = bounded(0);
    let h = tokio::spawn(async move {
        for i in 0..100 {
            tx.send_async(i).await.unwrap();
        }
    });
    for i in 0..100 {
        assert_eq!(rx.recv_async().await, Ok(i));
    }
    h.await.unwrap();
}

/// Benchmark-shaped stress test (SPSC async-async, b100, 16KB messages):
/// every message is delivered exactly once under the park/wake dynamics that
/// trigger the watermark confirm path.
#[tokio::test]
async fn spsc_b100_async_async_stress() {
    let (tx, rx) = bounded::<Vec<u8>>(100);
    const N: usize = 5000;
    const SIZE: usize = 16384;

    let sender = tokio::spawn(async move {
        for _ in 0..N {
            tx.send_async(vec![0u8; SIZE]).await.unwrap();
        }
    });
    let mut received = 0;
    while let Ok(msg) = rx.recv_async().await {
        assert_eq!(msg.len(), SIZE);
        received += 1;
    }
    assert_eq!(received, N);
    sender.await.unwrap();
}
