use loole as flume;

#[test]
fn same_sender() {
    let (tx1, _rx) = flume::unbounded::<()>();
    let tx2 = tx1.clone();

    assert!(tx1.same_channel(&tx2));

    let (tx3, _rx) = flume::unbounded::<()>();

    assert!(!tx1.same_channel(&tx3));
    assert!(!tx2.same_channel(&tx3));
}

#[test]
fn same_receiver() {
    let (_tx, rx1) = flume::unbounded::<()>();
    let rx2 = rx1.clone();

    assert!(rx1.same_channel(&rx2));

    let (_tx, rx3) = flume::unbounded::<()>();

    assert!(!rx1.same_channel(&rx3));
    assert!(!rx2.same_channel(&rx3));
}
