use loole::bounded;

#[tokio::test]
async fn async_select_recv_buffer_0() {
    let count = 10_000;

    let (tx1, rx1) = bounded(0);
    let (tx2, rx2) = bounded(0);

    tokio::spawn(async move {
        for i in (0..count).filter(|n| n % 2 == 0) {
            tx1.send_async(i).await.unwrap();
        }
    });
    tokio::spawn(async move {
        for i in (0..count).filter(|n| n % 2 == 1) {
            tx2.send_async(i).await.unwrap();
        }
    });

    let mut result = Vec::new();
    loop {
        let n = tokio::select! {
            n = rx1.recv_async() => n,
            n = rx2.recv_async() => n,
        };
        if let Ok(n) = n {
            result.push(n);
            if result.len() == count {
                break;
            }
        }
    }
    result.sort();
    let expected = (0..count).collect::<Vec<_>>();
    assert_eq!(result.len(), expected.len());
    assert_eq!(result, expected);
}
