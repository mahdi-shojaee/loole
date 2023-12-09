use std::time::{Duration, Instant};

use futures::{future::join_all, FutureExt};
use loole::{bounded, RecvError, SendError};

fn ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

async fn async_sleep(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await
}

#[tokio::test]
async fn async_send_before_recv_buffer_0() {
    let (tx, rx) = bounded(0);
    tokio::spawn(tx.send_async(1));
    async_sleep(100).await;
    assert_eq!(rx.recv_async().await, Ok(1));
}

#[tokio::test]
async fn async_recv_before_send_buffer_0() {
    let (tx, rx) = bounded(0);
    let h = tokio::spawn(rx.recv_async());
    async_sleep(100).await;
    let f = h.map(|x| x.unwrap());
    assert_eq!(tx.send_async(1).await, Ok(()));
    assert_eq!(f.await, Ok(1));
}

#[tokio::test]
async fn async_close_before_send_buffer_0() {
    let (tx, rx) = bounded::<()>(0);
    drop(rx);
    assert_eq!(tx.send_async(()).await, Err(SendError(())));
}

#[tokio::test]
async fn async_send_before_recv_buffer_1() {
    let (tx, rx) = bounded(1);
    assert_eq!(tx.send_async(1).await, Ok(()));
    assert_eq!(rx.recv_async().await, Ok(1));
}

#[tokio::test]
async fn async_recv_before_send_buffer_1() {
    let (tx, rx) = bounded(1);
    let h = tokio::spawn(rx.recv_async());
    async_sleep(100).await;
    assert_eq!(tx.send_async(1).await, Ok(()));
    assert_eq!(h.await.unwrap(), Ok(1));
}

#[tokio::test]
async fn async_2_sends_before_2_recvs_buffer_1() {
    let (tx, rx) = bounded(1);
    assert_eq!(tx.capacity(), Some(1));
    assert_eq!(tx.len(), 0);
    tx.send_async(1).await.unwrap();
    assert_eq!(tx.len(), 1);
    tokio::spawn(tx.send_async(2));
    async_sleep(100).await;
    assert_eq!(rx.len(), 1);
    let r1r = rx.recv_async().await;
    assert_eq!(r1r, Ok(1));
    assert_eq!(rx.len(), 1);
    let r2r = rx.recv_async().await;
    assert_eq!(r2r, Ok(2));
    assert_eq!(rx.len(), 0);
}

#[tokio::test]
async fn async_close_before_recv_buffer_0() {
    let (tx, rx) = bounded::<()>(0);
    drop(tx);
    assert_eq!(rx.recv_async().await, Err(RecvError::Disconnected));
}

#[tokio::test]
async fn async_close_before_recv_buffer_1() {
    let (tx, rx) = bounded(1);
    assert_eq!(tx.send_async(1).await, Ok(()));
    drop(tx);
    assert_eq!(rx.recv_async().await, Ok(1));
    assert_eq!(rx.recv_async().await, Err(RecvError::Disconnected));
}

#[tokio::test]
async fn async_concurrent_writes_and_reads_buffer_0() {
    let (tx, rx) = bounded(0);
    let _sends = tokio::spawn(join_all([
        tx.send_async(1),
        tx.send_async(2),
        tx.send_async(3),
    ]));
    let recvs = tokio::spawn(join_all([
        rx.recv_async(),
        rx.recv_async(),
        rx.recv_async(),
    ]))
    .await
    .unwrap();
    assert_eq!(recvs, vec![Ok(1), Ok(2), Ok(3)]);
}

#[tokio::test]
async fn async_shift_pending_send_buffer_0() {
    let (tx, rx) = bounded(0);
    let tx_clone = tx.clone();
    let h1 = tokio::spawn(async move {
        let start = Instant::now();
        assert_eq!(tx.send_async(1).await, Ok(()));
        start.elapsed()
    });
    let h2 = tokio::spawn(async move {
        let start = Instant::now();
        async_sleep(100).await;
        assert_eq!(tx_clone.send_async(2).await, Ok(()));
        start.elapsed()
    });
    tokio::spawn(async move {
        async_sleep(1000).await;
        assert_eq!(rx.recv(), Ok(1));
        async_sleep(1000).await;
        assert_eq!(rx.recv(), Ok(2));
    });
    let elapsed1 = h1.await.unwrap();
    assert!(elapsed1 >= ms(900));
    assert!(elapsed1 < ms(1100));
    let elapsed2 = h2.await.unwrap();
    assert!(elapsed2 >= ms(1900));
    assert!(elapsed2 < ms(2100));
}

#[tokio::test]
async fn async_shift_pending_send_buffer_2() {
    let (tx, rx) = bounded(2);
    assert_eq!(tx.send_async(1).await, Ok(()));
    assert_eq!(tx.send_async(2).await, Ok(()));
    let h = tokio::spawn(async move {
        let start = Instant::now();
        assert_eq!(tx.send_async(3).await, Ok(()));
        start.elapsed()
    });
    tokio::spawn(async move {
        async_sleep(1000).await;
        assert_eq!(rx.recv_async().await, Ok(1));
    })
    .await
    .unwrap();

    let elapsed = h.await.unwrap();
    assert!(elapsed >= ms(900));
    assert!(elapsed < ms(1100));
}
