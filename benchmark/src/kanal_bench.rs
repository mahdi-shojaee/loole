#![allow(clippy::manual_async_fn)] // LatencyChannel::send_async/recv_async must stay Send (tokio::spawn); async fn can't express that.
use kanal::{
    bounded, unbounded, AsyncReceiver, AsyncSender, ReceiveError, Receiver, SendError, Sender,
};
use std::{future::Future, thread};

use crate::bench_utils::{
    ping_pong_latency, setup_ping_pong, BenchError, Combo, JoinHandle, LatencyChannel,
    LatencyChannels, LatencyProbe, LatencyResult, ThroughputRun,
};
use crate::message_type::Message;
use tokio::runtime::Runtime;

pub enum KanalSender<T> {
    Sync(Sender<T>),
    Async(AsyncSender<T>),
}

pub enum KanalReceiver<T> {
    Sync(Receiver<T>),
    Async(AsyncReceiver<T>),
}

pub fn send<T>(tx: &Sender<T>, msg: T) -> Result<(), SendError> {
    tx.send(msg)
}

pub fn recv<T>(rx: &Receiver<T>) -> Result<T, ReceiveError> {
    rx.recv()
}

pub fn send_async<T: 'static>(
    tx: &AsyncSender<T>,
    msg: T,
) -> impl Future<Output = Result<(), SendError>> + '_ {
    tx.send(msg)
}

pub fn recv_async<T>(rx: &AsyncReceiver<T>) -> impl Future<Output = Result<T, ReceiveError>> + '_ {
    rx.recv()
}

fn setup_helper<S, R>(
    senders_no: usize,
    receivers_no: usize,
    cap: Option<usize>,
    msg_no: usize,
    message_size: usize,
    create_sender: S,
    create_receiver: R,
) -> Result<ThroughputRun, BenchError>
where
    S: Fn(Sender<Message>, usize, usize) -> JoinHandle<usize>,
    R: Fn(Receiver<Message>) -> JoinHandle<usize>,
{
    let (tx, rx) = cap.map_or_else(unbounded::<Message>, bounded);
    let senders = create_senders(tx, senders_no, msg_no, message_size, create_sender);
    let receivers = create_receivers(rx, receivers_no, create_receiver);
    Ok(ThroughputRun { senders, receivers })
}

fn create_senders<S>(
    tx: Sender<Message>,
    senders_no: usize,
    msg_no: usize,
    message_size: usize,
    create_sender: S,
) -> Vec<JoinHandle<usize>>
where
    S: Fn(Sender<Message>, usize, usize) -> JoinHandle<usize>,
{
    let mut senders = Vec::with_capacity(senders_no);
    for i in 0..senders_no {
        let n = msg_no / senders_no + if i < msg_no % senders_no { 1 } else { 0 };
        senders.push(create_sender(tx.clone(), n, message_size));
    }
    senders
}

fn create_receivers<R>(
    rx: Receiver<Message>,
    receivers_no: usize,
    create_receiver: R,
) -> Vec<JoinHandle<usize>>
where
    R: Fn(Receiver<Message>) -> JoinHandle<usize>,
{
    let mut receivers = Vec::with_capacity(receivers_no);
    for _ in 0..receivers_no {
        receivers.push(create_receiver(rx.clone()));
    }
    receivers
}

fn create_sync_sender(tx: Sender<Message>, n: usize, message_size: usize) -> JoinHandle<usize> {
    JoinHandle::Sync(thread::spawn(move || {
        let mut error_count = 0;
        let mut sent_count = 0;
        for _ in 0..n {
            match send(&tx, Message::new(message_size)) {
                Ok(_) => sent_count += 1,
                Err(_) => error_count += 1,
            }
        }
        if error_count > 0 {
            println!("Sync sender encountered {} errors", error_count);
        }
        sent_count
    }))
}

fn create_async_sender(tx: Sender<Message>, n: usize, message_size: usize) -> JoinHandle<usize> {
    let tx = tx.to_async();
    JoinHandle::Async(tokio::spawn(async move {
        let mut error_count = 0;
        let mut sent_count = 0;
        for _ in 0..n {
            match send_async(&tx, Message::new(message_size)).await {
                Ok(_) => sent_count += 1,
                Err(_) => error_count += 1,
            }
        }
        if error_count > 0 {
            println!("Async sender encountered {} errors", error_count);
        }
        sent_count
    }))
}

fn create_sync_receiver(rx: Receiver<Message>) -> JoinHandle<usize> {
    JoinHandle::Sync(thread::spawn(move || {
        let mut received_count = 0;
        while recv(&rx).is_ok() {
            received_count += 1;
        }
        received_count
    }))
}

fn create_async_receiver(rx: Receiver<Message>) -> JoinHandle<usize> {
    let rx = rx.to_async();
    JoinHandle::Async(tokio::spawn(async move {
        let mut received_count = 0;
        while recv_async(&rx).await.is_ok() {
            received_count += 1;
        }
        received_count
    }))
}

pub fn setup_sync_sync(
    senders_no: usize,
    receivers_no: usize,
    buffer_size: Option<usize>,
    msg_no: usize,
    message_size: usize,
) -> Result<ThroughputRun, BenchError> {
    setup_helper(
        senders_no,
        receivers_no,
        buffer_size,
        msg_no,
        message_size,
        create_sync_sender,
        create_sync_receiver,
    )
}

pub fn setup_async_async(
    senders_no: usize,
    receivers_no: usize,
    buffer_size: Option<usize>,
    msg_no: usize,
    message_size: usize,
) -> Result<ThroughputRun, BenchError> {
    setup_helper(
        senders_no,
        receivers_no,
        buffer_size,
        msg_no,
        message_size,
        create_async_sender,
        create_async_receiver,
    )
}

pub fn setup_async_sync(
    senders_no: usize,
    receivers_no: usize,
    buffer_size: Option<usize>,
    msg_no: usize,
    message_size: usize,
) -> Result<ThroughputRun, BenchError> {
    setup_helper(
        senders_no,
        receivers_no,
        buffer_size,
        msg_no,
        message_size,
        create_async_sender,
        create_sync_receiver,
    )
}

pub fn setup_sync_async(
    senders_no: usize,
    receivers_no: usize,
    buffer_size: Option<usize>,
    msg_no: usize,
    message_size: usize,
) -> Result<ThroughputRun, BenchError> {
    setup_helper(
        senders_no,
        receivers_no,
        buffer_size,
        msg_no,
        message_size,
        create_sync_sender,
        create_async_receiver,
    )
}

pub struct KanalChannel;

impl LatencyChannel<Message> for KanalChannel {
    type Sender = KanalSender<Message>;
    type Receiver = KanalReceiver<Message>;

    fn validate(_combo: Combo, _cap: Option<usize>) -> Result<(), BenchError> {
        Ok(())
    }

    fn create(combo: Combo, cap: Option<usize>) -> LatencyChannels<Self, Message> {
        let pair = || cap.map_or_else(unbounded::<Message>, bounded);
        let (ftx, frx) = pair();
        let (btx, brx) = pair();
        let sender_async = matches!(combo, Combo::AsyncAsync | Combo::AsyncSync);
        let echoer_async = matches!(combo, Combo::AsyncAsync | Combo::SyncAsync);
        let sender_tx = if sender_async {
            KanalSender::Async(ftx.to_async())
        } else {
            KanalSender::Sync(ftx)
        };
        let sender_rx = if sender_async {
            KanalReceiver::Async(brx.to_async())
        } else {
            KanalReceiver::Sync(brx)
        };
        let echoer_rx = if echoer_async {
            KanalReceiver::Async(frx.to_async())
        } else {
            KanalReceiver::Sync(frx)
        };
        let echoer_tx = if echoer_async {
            KanalSender::Async(btx.to_async())
        } else {
            KanalSender::Sync(btx)
        };
        LatencyChannels {
            sender_tx,
            sender_rx,
            echoer_tx,
            echoer_rx,
        }
    }

    fn send(tx: &Self::Sender, msg: Message) {
        match tx {
            KanalSender::Sync(tx) => {
                let _ = tx.send(msg);
            }
            KanalSender::Async(_) => unreachable!("kanal sync send on async channel"),
        }
    }

    fn recv(rx: &mut Self::Receiver) -> Option<Message> {
        match rx {
            KanalReceiver::Sync(rx) => rx.recv().ok(),
            KanalReceiver::Async(_) => unreachable!("kanal sync recv on async channel"),
        }
    }

    fn send_async(tx: &Self::Sender, msg: Message) -> impl Future<Output = ()> + Send + '_ {
        async move {
            match tx {
                KanalSender::Async(tx) => {
                    let _ = send_async(tx, msg).await;
                }
                KanalSender::Sync(_) => unreachable!("kanal async send on sync channel"),
            }
        }
    }

    fn recv_async(rx: &mut Self::Receiver) -> impl Future<Output = Option<Message>> + Send + '_ {
        async move {
            match rx {
                KanalReceiver::Async(rx) => recv_async(rx).await.ok(),
                KanalReceiver::Sync(_) => unreachable!("kanal async recv on sync channel"),
            }
        }
    }
}

pub fn validate_latency(combo: Combo, cap: Option<usize>) -> Result<(), BenchError> {
    <KanalChannel as LatencyChannel<Message>>::validate(combo, cap)
}

pub fn setup_latency(
    rt: &Runtime,
    combo: Combo,
    cap: Option<usize>,
    message_size: usize,
    warmup: usize,
) -> Result<LatencyProbe, BenchError> {
    setup_ping_pong::<KanalChannel>(rt, combo, cap, message_size, warmup)
}

pub fn bench_latency(
    rt: &Runtime,
    combo: Combo,
    cap: Option<usize>,
    message_size: usize,
) -> Result<LatencyResult, BenchError> {
    ping_pong_latency::<KanalChannel>(rt, combo, cap, message_size)
}
