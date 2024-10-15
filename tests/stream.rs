use {loole::*, futures::StreamExt};

#[test]
fn stream_recv_drop_recv() {
    let (tx, rx) = bounded::<i32>(10);

    let rx2 = rx.clone();
    let mut stream = rx.into_stream();

    async_std::task::block_on(async {
        let res =
            async_std::future::timeout(std::time::Duration::from_millis(500), stream.next()).await;

        assert!(res.is_err());
    });

    let t =
        std::thread::spawn(move || async_std::task::block_on(async { rx2.stream().next().await }));

    std::thread::sleep(std::time::Duration::from_millis(500));

    tx.send(42).unwrap();

    drop(stream);

    assert_eq!(t.join().unwrap(), Some(42))
}

#[test]
fn stream_recv_not_drop_recv() {
    let (tx, rx) = bounded::<i32>(10);

    let rx2 = rx.clone();
    let mut stream = rx.into_stream();

    async_std::task::block_on(async {
        let res =
            async_std::future::timeout(std::time::Duration::from_millis(500), stream.next()).await;

        assert!(res.is_err());
    });

    let t =
        std::thread::spawn(move || async_std::task::block_on(async { rx2.stream().next().await }));

    std::thread::sleep(std::time::Duration::from_millis(500));

    tx.send(42).unwrap();

    assert_eq!(t.join().unwrap(), Some(42))
}
