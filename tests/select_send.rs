//! `select!` tests involving the send side and non-zero-capacity
//! channels, complementing `tests/select.rs` (which only selects over two
//! rendezvous receive futures).

use std::time::Duration;

use loole::{bounded, unbounded};

#[tokio::test]
async fn select_over_two_send_futures_on_full_channels() {
    // Two full channels: both send futures are pending, then space frees up
    // on channel 2, so its send completes first.
    let (tx1, rx1) = bounded::<u8>(1);
    let (tx2, rx2) = bounded::<u8>(1);
    tx1.send(10).unwrap();
    tx2.send(20).unwrap();

    let send1 = tx1.send_async(11);
    let send2 = tx2.send_async(21);

    // Free space on channel 2 only.
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(rx2.recv_async().await, Ok(20));
    });

    let completed_on_2 = tokio::select! {
        _ = send1 => false,
        _ = send2 => true,
    };
    assert!(completed_on_2, "the send on channel 2 should complete first");

    // The channel-1 send future was dropped and its message cancelled:
    // only the original buffered message remains on channel 1.
    assert_eq!(rx1.recv_async().await, Ok(10));
    assert_eq!(rx1.try_recv(), Err(loole::TryRecvError::Empty));
}

#[tokio::test]
async fn select_send_vs_timeout() {
    // A send on a full channel loses to a timeout branch; the dropped future's
    // message is cancelled.
    let (tx, rx) = bounded(1);
    tx.send(1).unwrap();

    let send_fut = tx.send_async(2);
    tokio::select! {
        _ = send_fut => panic!("send should not complete while full"),
        _ = tokio::time::sleep(Duration::from_millis(50)) => {}
    }

    assert_eq!(rx.recv_async().await, Ok(1));
    assert_eq!(rx.try_recv(), Err(loole::TryRecvError::Empty));
}

#[tokio::test]
async fn select_recv_on_nonzero_capacity_channels() {
    // `tests/select.rs` covers rendezvous channels; this selects over two
    // buffered channels. Only one has a message, so its branch must win
    // deterministically.
    let (tx1, rx1) = bounded::<u8>(4);
    let (tx2, rx2) = bounded::<u8>(4);
    tx1.send(1).unwrap();

    let first = tokio::select! {
        v = rx1.recv_async() => v.unwrap(),
        v = rx2.recv_async() => v.unwrap(),
    };
    assert_eq!(first, 1);

    // Now rx1 is empty and a message arrives on rx2; its branch wins.
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        tx2.send(7).unwrap();
    });
    let second = tokio::select! {
        v = rx1.recv_async() => v.unwrap(),
        v = rx2.recv_async() => v.unwrap(),
    };
    assert_eq!(second, 7);
    assert_eq!(rx1.try_recv(), Err(loole::TryRecvError::Empty));
}

#[tokio::test]
async fn select_send_error_branch() {
    // A send future on a disconnected channel resolves with an error; the
    // select must be able to take that branch.
    let (tx, rx) = bounded::<u8>(1);
    drop(rx);

    tokio::select! {
        res = tx.send_async(1) => {
            assert_eq!(res, Err(loole::SendError(1)));
        }
        _ = tokio::time::sleep(Duration::from_millis(500)) => panic!("send should fail fast on a closed channel"),
    }
}

#[tokio::test]
async fn select_stream_and_recv_async() {
    // Select over a `RecvStream` (via `next()`) and a `RecvFuture` on two
    // unbounded channels. Both channels receive all 10 messages, though not
    // necessarily in lockstep (a branch may win several times in a row).
    use futures::StreamExt;

    const COUNT: usize = 10;

    let (tx1, rx1) = unbounded::<usize>();
    let (tx2, rx2) = unbounded::<usize>();

    tokio::spawn(async move {
        for i in 0..COUNT {
            tx1.send(i).unwrap();
            tx2.send(i).unwrap();
        }
    });

    let mut stream = rx1.stream();
    let mut stream_items: Vec<usize> = Vec::new();
    let mut recv_items: Vec<usize> = Vec::new();
    while stream_items.len() < COUNT || recv_items.len() < COUNT {
        tokio::select! {
            v = stream.next() => if let Some(v) = v { stream_items.push(v); },
            v = rx2.recv_async() => if let Ok(v) = v { recv_items.push(v); },
        }
    }
    let expected: Vec<usize> = (0..COUNT).collect();
    assert_eq!(stream_items, expected);
    assert_eq!(recv_items, expected);
}

#[tokio::test]
async fn select_cancellation_under_load_stays_consistent() {
    // Stress variant of `select_send_vs_timeout`: repeatedly cancel sends on a
    // full channel. Each cancelled message is either delivered (it won the
    // hand-off race before cancellation) or destroyed — but the received
    // sequence must be strictly ordered 0, 1, 2, ... with no duplicates and
    // no gaps between *received* values (cancelled ones may be skipped only
    // if the consumer still terminates).
    const COUNT: usize = 200;

    let (tx, rx) = bounded(1);
    tx.send(0).unwrap();

    let consumer = tokio::spawn(async move {
        let mut received = Vec::new();
        // Drain until the producer closes the channel; cancellations may
        // remove values, so we cannot expect COUNT+1 messages.
        loop {
            match rx.recv_async().await {
                Ok(v) => received.push(v),
                Err(_) => break,
            }
        }
        received
    });

    for i in 1..=COUNT {
        let send_fut = tx.send_async(i);
        tokio::select! {
            _ = send_fut => {}
            _ = tokio::time::sleep(Duration::from_nanos(1)) => {}
        }
    }
    drop(tx);

    let received = consumer.await.unwrap();
    // Whatever arrived must be a strictly increasing subsequence of 0..=COUNT:
    // order preserved, no duplicates, no reordering.
    assert!(!received.is_empty(), "at least the first message must arrive");
    let expected: Vec<usize> = (0..=COUNT).collect();
    let mut it = received.iter();
    let first = it.next().unwrap();
    assert_eq!(*first, 0, "message 0 was never cancelled, must arrive first");
    let mut prev = 0;
    for v in it {
        let idx = expected.binary_search(v).unwrap();
        assert!(idx > prev, "values must be in order, no duplicates");
        prev = idx;
    }
}
