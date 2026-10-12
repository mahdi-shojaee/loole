#![allow(clippy::manual_async_fn)] // LatencyChannel::send_async/recv_async must stay Send (tokio::spawn); async fn can't express that.
use flume::{bounded, unbounded, Receiver, RecvError, SendError, Sender};
use std::{future::Future, thread};

use crate::bench_utils::{
    ping_pong_latency, setup_ping_pong, BenchError, Combo, JoinHandle, LatencyChannel,
    LatencyChannels, LatencyProbe, LatencyResult, ThroughputRun,
};
use crate::message_type::{AnyPayload, Message, PayloadKind};
use tokio::runtime::Runtime;

pub fn send(tx: &Sender<AnyPayload>, msg: AnyPayload) -> Result<(), SendError<AnyPayload>> {
    tx.send(msg)
}

pub fn recv(rx: &Receiver<AnyPayload>) -> Result<AnyPayload, RecvError> {
    rx.recv()
}

pub fn send_async(
    tx: &Sender<AnyPayload>,
    msg: AnyPayload,
) -> impl Future<Output = Result<(), SendError<AnyPayload>>> + '_ {
    tx.send_async(msg)
}

pub fn recv_async(rx: &Receiver<AnyPayload>) -> impl Future<Output = Result<AnyPayload, RecvError>> + '_ {
    rx.recv_async()
}

fn setup_helper<S, R>(
    senders_no: usize,
    receivers_no: usize,
    cap: Option<usize>,
    msg_no: usize,
    message_size: usize,
    payload: PayloadKind,
    create_sender: S,
    create_receiver: R,
) -> Result<ThroughputRun, BenchError>
where
    S: Fn(Sender<AnyPayload>, usize, usize, PayloadKind) -> JoinHandle<usize>,
    R: Fn(Receiver<AnyPayload>) -> JoinHandle<usize>,
{
    let (tx, rx) = cap.map_or_else(unbounded::<AnyPayload>, bounded);
    let senders = create_senders(tx, senders_no, msg_no, message_size, payload, create_sender);
    let receivers = create_receivers(rx, receivers_no, create_receiver);
    Ok(ThroughputRun { senders, receivers })
}

fn create_senders<S>(
    tx: Sender<AnyPayload>,
    senders_no: usize,
    msg_no: usize,
    message_size: usize,
    payload: PayloadKind,
    create_sender: S,
) -> Vec<JoinHandle<usize>>
where
    S: Fn(Sender<AnyPayload>, usize, usize, PayloadKind) -> JoinHandle<usize>,
{
    let mut senders = Vec::with_capacity(senders_no);
    for i in 0..senders_no {
        let n = msg_no / senders_no + if i < msg_no % senders_no { 1 } else { 0 };
        senders.push(create_sender(tx.clone(), n, message_size, payload));
    }
    senders
}

fn create_receivers<R>(
    rx: Receiver<AnyPayload>,
    receivers_no: usize,
    create_receiver: R,
) -> Vec<JoinHandle<usize>>
where
    R: Fn(Receiver<AnyPayload>) -> JoinHandle<usize>,
{
    let mut receivers = Vec::with_capacity(receivers_no);
    for _ in 0..receivers_no {
        receivers.push(create_receiver(rx.clone()));
    }
    receivers
}

fn create_sync_sender(
    tx: Sender<AnyPayload>,
    n: usize,
    message_size: usize,
    payload: PayloadKind,
) -> JoinHandle<usize> {
    JoinHandle::Sync(thread::spawn(move || {
        let mut error_count = 0;
        let mut sent_count = 0;
        let mut cache = None;
        for _ in 0..n {
            match send(&tx, AnyPayload::fresh(payload, message_size, &mut cache)) {
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

fn create_async_sender(
    tx: Sender<AnyPayload>,
    n: usize,
    message_size: usize,
    payload: PayloadKind,
) -> JoinHandle<usize> {
    JoinHandle::Async(tokio::spawn(async move {
        let mut error_count = 0;
        let mut sent_count = 0;
        let mut cache = None;
        for _ in 0..n {
            match send_async(&tx, AnyPayload::fresh(payload, message_size, &mut cache)).await {
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

fn create_sync_receiver(rx: Receiver<AnyPayload>) -> JoinHandle<usize> {
    JoinHandle::Sync(thread::spawn(move || {
        let mut received_count = 0;
        while recv(&rx).is_ok() {
            received_count += 1;
        }
        received_count
    }))
}

fn create_async_receiver(rx: Receiver<AnyPayload>) -> JoinHandle<usize> {
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
    payload: PayloadKind,
) -> Result<ThroughputRun, BenchError> {
    setup_helper(
        senders_no,
        receivers_no,
        buffer_size,
        msg_no,
        message_size,
        payload,
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
    payload: PayloadKind,
) -> Result<ThroughputRun, BenchError> {
    setup_helper(
        senders_no,
        receivers_no,
        buffer_size,
        msg_no,
        message_size,
        payload,
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
    payload: PayloadKind,
) -> Result<ThroughputRun, BenchError> {
    setup_helper(
        senders_no,
        receivers_no,
        buffer_size,
        msg_no,
        message_size,
        payload,
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
    payload: PayloadKind,
) -> Result<ThroughputRun, BenchError> {
    setup_helper(
        senders_no,
        receivers_no,
        buffer_size,
        msg_no,
        message_size,
        payload,
        create_sync_sender,
        create_async_receiver,
    )
}

pub struct FlumeChannel;

impl LatencyChannel<Message> for FlumeChannel {
    type Sender = Sender<Message>;
    type Receiver = Receiver<Message>;

    fn validate(_combo: Combo, _cap: Option<usize>) -> Result<(), BenchError> {
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
        let _ = tx.send(msg);
    }

    fn recv(rx: &mut Self::Receiver) -> Option<Message> {
        rx.recv().ok()
    }

    fn send_async(tx: &Self::Sender, msg: Message) -> impl Future<Output = ()> + Send + '_ {
        async move {
            let _ = tx.send_async(msg).await;
        }
    }

    fn recv_async(rx: &mut Self::Receiver) -> impl Future<Output = Option<Message>> + Send + '_ {
        async move { rx.recv_async().await.ok() }
    }
}

pub fn validate_latency(combo: Combo, cap: Option<usize>) -> Result<(), BenchError> {
    <FlumeChannel as LatencyChannel<Message>>::validate(combo, cap)
}

pub fn setup_latency(
    rt: &Runtime,
    combo: Combo,
    cap: Option<usize>,
    message_size: usize,
    warmup: usize,
    payload: PayloadKind,
) -> Result<LatencyProbe, BenchError> {
    setup_ping_pong::<FlumeChannel>(rt, combo, cap, message_size, warmup, payload)
}

pub fn bench_latency(
    rt: &Runtime,
    combo: Combo,
    cap: Option<usize>,
    message_size: usize,
    payload: PayloadKind,
) -> Result<LatencyResult, BenchError> {
    ping_pong_latency::<FlumeChannel>(rt, combo, cap, message_size, payload)
}
