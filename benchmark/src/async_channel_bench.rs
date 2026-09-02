#![allow(clippy::manual_async_fn)] // LatencyChannel::send_async/recv_async must stay Send (tokio::spawn); async fn can't express that.
use async_channel::{bounded, unbounded, Receiver, RecvError, SendError, Sender};
use std::{future::Future, thread};

use crate::bench_utils::{
    ping_pong_latency, setup_ping_pong, BenchError, Combo, JoinHandle, LatencyChannel,
    LatencyChannels, LatencyProbe, LatencyResult, ThroughputRun,
};
use crate::message_type::Message;
use tokio::runtime::Runtime;

pub fn send(tx: &Sender<Message>, msg: Message) -> Result<(), SendError<Message>> {
    tx.send_blocking(msg)
}

pub fn recv(rx: &Receiver<Message>) -> Result<Message, RecvError> {
    rx.recv_blocking()
}

pub fn send_async(
    tx: &Sender<Message>,
    msg: Message,
) -> impl Future<Output = Result<(), SendError<Message>>> + '_ {
    tx.send(msg)
}

pub fn recv_async(rx: &Receiver<Message>) -> impl Future<Output = Result<Message, RecvError>> + '_ {
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
    if cap == Some(0) {
        return Err(BenchError::ZeroCapacityNotSupported);
    }
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

pub struct AsyncChannelChannel;

impl LatencyChannel<Message> for AsyncChannelChannel {
    type Sender = Sender<Message>;
    type Receiver = Receiver<Message>;

    fn validate(_combo: Combo, cap: Option<usize>) -> Result<(), BenchError> {
        if cap == Some(0) {
            return Err(BenchError::ZeroCapacityNotSupported);
        }
        Ok(())
    }

    fn create(_combo: Combo, cap: Option<usize>) -> LatencyChannels<Self, Message> {
        let pair = || cap.map_or_else(unbounded::<Message>, bounded);
        let (ftx, frx) = pair();
        let (btx, brx) = pair();
        LatencyChannels {
            sender_tx: ftx,
            sender_rx: brx,
            echoer_tx: btx,
            echoer_rx: frx,
        }
    }

    fn send(tx: &Self::Sender, msg: Message) {
        let _ = tx.send_blocking(msg);
    }

    fn recv(rx: &mut Self::Receiver) -> Option<Message> {
        rx.recv_blocking().ok()
    }

    fn send_async(tx: &Self::Sender, msg: Message) -> impl Future<Output = ()> + Send + '_ {
        async move {
            let _ = tx.send(msg).await;
        }
    }

    fn recv_async(rx: &mut Self::Receiver) -> impl Future<Output = Option<Message>> + Send + '_ {
        async move { rx.recv().await.ok() }
    }
}

pub fn validate_latency(combo: Combo, cap: Option<usize>) -> Result<(), BenchError> {
    <AsyncChannelChannel as LatencyChannel<Message>>::validate(combo, cap)
}

pub fn setup_latency(
    rt: &Runtime,
    combo: Combo,
    cap: Option<usize>,
    message_size: usize,
    warmup: usize,
) -> Result<LatencyProbe, BenchError> {
    setup_ping_pong::<AsyncChannelChannel>(rt, combo, cap, message_size, warmup)
}

pub fn bench_latency(
    rt: &Runtime,
    combo: Combo,
    cap: Option<usize>,
    message_size: usize,
) -> Result<LatencyResult, BenchError> {
    ping_pong_latency::<AsyncChannelChannel>(rt, combo, cap, message_size)
}
