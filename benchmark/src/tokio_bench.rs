#![allow(clippy::manual_async_fn)] // LatencyChannel::send_async/recv_async must stay Send (tokio::spawn); async fn can't express that.
use std::{future::Future, thread};
use tokio::sync::mpsc::{
    channel, error::SendError, unbounded_channel, Receiver, Sender, UnboundedReceiver,
    UnboundedSender,
};

use crate::bench_utils::{
    ping_pong_latency, setup_ping_pong, BenchError, Combo, JoinHandle, LatencyChannel,
    LatencyChannels, LatencyProbe, LatencyResult, ThroughputRun,
};
use crate::message_type::{AnyPayload, Message, PayloadKind};
use tokio::runtime::Runtime;

pub enum SenderEnum<T> {
    Bounded(Sender<T>),
    Unbounded(UnboundedSender<T>),
}

pub enum ReceiverEnum<T> {
    Bounded(Receiver<T>),
    Unbounded(UnboundedReceiver<T>),
}

impl<T> Clone for SenderEnum<T> {
    fn clone(&self) -> Self {
        match self {
            SenderEnum::Bounded(tx) => SenderEnum::Bounded(tx.clone()),
            SenderEnum::Unbounded(tx) => SenderEnum::Unbounded(tx.clone()),
        }
    }
}

pub fn send<T>(tx: &SenderEnum<T>, msg: T) -> Result<(), SendError<T>> {
    match tx {
        SenderEnum::Bounded(tx) => tx.blocking_send(msg),
        SenderEnum::Unbounded(tx) => tx.send(msg),
    }
}

pub fn recv<T>(rx: &mut ReceiverEnum<T>) -> Option<T> {
    match rx {
        ReceiverEnum::Bounded(rx) => rx.blocking_recv(),
        ReceiverEnum::Unbounded(rx) => rx.blocking_recv(),
    }
}

pub async fn send_async<T: 'static>(tx: &SenderEnum<T>, msg: T) -> Result<(), SendError<T>> {
    match tx {
        SenderEnum::Bounded(tx) => tx.send(msg).await,
        SenderEnum::Unbounded(tx) => tx.send(msg),
    }
}

pub async fn recv_async<T>(rx: &mut ReceiverEnum<T>) -> Option<T> {
    match rx {
        ReceiverEnum::Bounded(rx) => rx.recv().await,
        ReceiverEnum::Unbounded(rx) => rx.recv().await,
    }
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
    S: Fn(SenderEnum<AnyPayload>, usize, usize, PayloadKind) -> JoinHandle<usize>,
    R: Fn(ReceiverEnum<AnyPayload>) -> JoinHandle<usize>,
{
    if receivers_no > 1 {
        return Err(BenchError::MpmcNotSupported);
    }
    if cap == Some(0) {
        return Err(BenchError::ZeroCapacityNotSupported);
    }
    let (tx, rx) = match cap {
        Some(cap) => {
            let (tx, rx) = channel(cap);
            (SenderEnum::Bounded(tx), ReceiverEnum::Bounded(rx))
        }
        None => {
            let (tx, rx) = unbounded_channel();
            (SenderEnum::Unbounded(tx), ReceiverEnum::Unbounded(rx))
        }
    };
    let senders = create_senders(tx, senders_no, msg_no, message_size, payload, create_sender);
    let receivers = create_receivers(rx, receivers_no, create_receiver);
    Ok(ThroughputRun { senders, receivers })
}

fn create_senders<S>(
    tx: SenderEnum<AnyPayload>,
    senders_no: usize,
    msg_no: usize,
    message_size: usize,
    payload: PayloadKind,
    create_sender: S,
) -> Vec<JoinHandle<usize>>
where
    S: Fn(SenderEnum<AnyPayload>, usize, usize, PayloadKind) -> JoinHandle<usize>,
{
    let mut senders = Vec::with_capacity(senders_no);
    for i in 0..senders_no {
        let n = msg_no / senders_no + if i < msg_no % senders_no { 1 } else { 0 };
        senders.push(create_sender(tx.clone(), n, message_size, payload));
    }
    senders
}

fn create_receivers<R>(
    rx: ReceiverEnum<AnyPayload>,
    receivers_no: usize,
    create_receiver: R,
) -> Vec<JoinHandle<usize>>
where
    R: Fn(ReceiverEnum<AnyPayload>) -> JoinHandle<usize>,
{
    let mut receivers = Vec::with_capacity(receivers_no);
    receivers.push(create_receiver(rx));
    receivers
}

fn create_sync_sender(
    tx: SenderEnum<AnyPayload>,
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
    tx: SenderEnum<AnyPayload>,
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

fn create_sync_receiver(mut rx: ReceiverEnum<AnyPayload>) -> JoinHandle<usize> {
    JoinHandle::Sync(thread::spawn(move || {
        let mut received_count = 0;
        while recv(&mut rx).is_some() {
            received_count += 1;
        }
        received_count
    }))
}

fn create_async_receiver(mut rx: ReceiverEnum<AnyPayload>) -> JoinHandle<usize> {
    JoinHandle::Async(tokio::spawn(async move {
        let mut received_count = 0;
        while recv_async(&mut rx).await.is_some() {
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

pub struct TokioChannel;

impl LatencyChannel<Message> for TokioChannel {
    type Sender = SenderEnum<Message>;
    type Receiver = ReceiverEnum<Message>;

    fn validate(_combo: Combo, cap: Option<usize>) -> Result<(), BenchError> {
        if cap == Some(0) {
            return Err(BenchError::ZeroCapacityNotSupported);
        }
        Ok(())
    }

    fn create(_combo: Combo, cap: Option<usize>) -> LatencyChannels<Self, Message> {
        let pair = || match cap {
            Some(cap) => {
                let (tx, rx) = channel(cap);
                (SenderEnum::Bounded(tx), ReceiverEnum::Bounded(rx))
            }
            None => {
                let (tx, rx) = unbounded_channel();
                (SenderEnum::Unbounded(tx), ReceiverEnum::Unbounded(rx))
            }
        };
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
        let _ = send(tx, msg);
    }

    fn recv(rx: &mut Self::Receiver) -> Option<Message> {
        recv(rx)
    }

    fn send_async(tx: &Self::Sender, msg: Message) -> impl Future<Output = ()> + Send + '_ {
        let tx = tx.clone();
        async move {
            let _ = send_async(&tx, msg).await;
        }
    }

    fn recv_async(rx: &mut Self::Receiver) -> impl Future<Output = Option<Message>> + Send + '_ {
        async move { recv_async(rx).await }
    }
}

pub fn validate_latency(combo: Combo, cap: Option<usize>) -> Result<(), BenchError> {
    <TokioChannel as LatencyChannel<Message>>::validate(combo, cap)
}

pub fn setup_latency(
    rt: &Runtime,
    combo: Combo,
    cap: Option<usize>,
    message_size: usize,
    warmup: usize,
) -> Result<LatencyProbe, BenchError> {
    setup_ping_pong::<TokioChannel>(rt, combo, cap, message_size, warmup)
}

pub fn bench_latency(
    rt: &Runtime,
    combo: Combo,
    cap: Option<usize>,
    message_size: usize,
) -> Result<LatencyResult, BenchError> {
    ping_pong_latency::<TokioChannel>(rt, combo, cap, message_size)
}
