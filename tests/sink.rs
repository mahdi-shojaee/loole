use loole::*;
use futures::SinkExt;
use futures::StreamExt;
use std::time::Duration;

#[tokio::test]
async fn test_send_sink_basic() {
    let (tx, rx) = unbounded();
    let mut sink = tx.into_sink();

    sink.send(1).await.unwrap();
    sink.send(2).await.unwrap();
    sink.send(3).await.unwrap();

    assert_eq!(rx.recv().unwrap(), 1);
    assert_eq!(rx.recv().unwrap(), 2);
    assert_eq!(rx.recv().unwrap(), 3);
}

#[tokio::test]
async fn test_send_sink_flush() {
    let (tx, rx) = bounded(2);
    let mut sink = tx.into_sink();

    sink.send(1).await.unwrap();
    sink.send(2).await.unwrap();

    // This send should block until we receive an item
    let send_future = sink.send(3);

    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(rx.recv().unwrap(), 1);

    send_future.await.unwrap();

    assert_eq!(rx.recv().unwrap(), 2);
    assert_eq!(rx.recv().unwrap(), 3);
}

#[tokio::test]
async fn test_send_sink_close() {
    let (tx, rx) = unbounded();
    let mut sink = tx.into_sink();

    sink.send(1).await.unwrap();
    sink.send(2).await.unwrap();
    sink.close().await.unwrap();

    assert_eq!(rx.recv().unwrap(), 1);
    assert_eq!(rx.recv().unwrap(), 2);
    assert!(rx.recv().is_err());
}

#[tokio::test]
async fn test_send_sink_multiple() {
    let (tx, rx) = unbounded();
    let mut sink = tx.into_sink();

    let items = [1, 2, 3, 4, 5];
    for item in items {
        sink.send(item).await.unwrap();
    }

    let received: Vec<i32> = rx.stream().take(5).collect().await;
    assert_eq!(received, vec![1, 2, 3, 4, 5]);
}

#[tokio::test]
async fn test_send_sink_error_propagation() {
    let (tx, rx) = bounded(2);
    let mut sink = tx.into_sink();

    sink.send(1).await.unwrap();
    sink.send(2).await.unwrap();

    // Close the receiver to cause a send error
    drop(rx);

    // This send should fail
    assert!(sink.send(3).await.is_err());
}

#[tokio::test]
async fn test_send_sink_clone() {
    let (tx, rx) = unbounded();
    let mut sink1 = tx.into_sink();
    let mut sink2 = sink1.clone();

    sink1.send(1).await.unwrap();
    sink2.send(2).await.unwrap();
    sink1.send(3).await.unwrap();

    assert_eq!(rx.recv().unwrap(), 1);
    assert_eq!(rx.recv().unwrap(), 2);
    assert_eq!(rx.recv().unwrap(), 3);
}
