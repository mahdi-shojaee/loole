#![allow(clippy::manual_async_fn)] // LatencyChannel::send_async/recv_async must stay Send (tokio::spawn); async fn can't express that.
use loole::{bounded, unbounded, Receiver, RecvError, SendError, Sender};
use std::{future::Future, thread};

use crate::bench_utils::{
    ping_pong_latency, setup_ping_pong, BenchError, Combo, JoinHandle, LatencyChannel,
    LatencyChannels, LatencyProbe, LatencyResult, ThroughputRun, record_stamp,
};
use crate::message_type::{AnyPayload, DistCtx, Message, PayloadKind};
use hdrhistogram::Histogram;
use tokio::runtime::Runtime;

pub fn send<T>(tx: &Sender<T>, msg: T) -> Result<(), SendError<T>> {
    tx.send(msg)
}

pub fn recv<T>(rx: &Receiver<T>) -> Result<T, RecvError> {
    rx.recv()
}

pub fn send_async<T: 'static>(
    tx: &Sender<T>,
    msg: T,
) -> impl Future<Output = Result<(), SendError<T>>> + '_ {
    tx.send_async(msg)
}

pub fn recv_async<T>(rx: &Receiver<T>) -> impl Future<Output = Result<T, RecvError>> + '_ {
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
    S: Fn(Sender<AnyPayload>, usize, usize, PayloadKind, DistCtx) -> JoinHandle<usize>,
    R: Fn(Receiver<AnyPayload>, DistCtx) -> JoinHandle<(usize, Histogram<u64>)>,
{
    let (tx, rx) = cap.map_or_else(unbounded::<AnyPayload>, bounded);
    let dctx = DistCtx::new(msg_no);
    let senders = create_senders(tx, senders_no, msg_no, message_size, payload, create_sender, dctx);
    let receivers = create_receivers(rx, receivers_no, create_receiver, dctx);
    Ok(ThroughputRun { senders, receivers })
}

fn create_senders<S>(
    tx: Sender<AnyPayload>,
    senders_no: usize,
    msg_no: usize,
    message_size: usize,
    payload: PayloadKind,
    create_sender: S,
    dctx: DistCtx,
) -> Vec<JoinHandle<usize>>
where
    S: Fn(Sender<AnyPayload>, usize, usize, PayloadKind, DistCtx) -> JoinHandle<usize>,
{
    let mut senders = Vec::with_capacity(senders_no);
    for i in 0..senders_no {
        let n = msg_no / senders_no + if i < msg_no % senders_no { 1 } else { 0 };
        senders.push(create_sender(tx.clone(), n, message_size, payload, dctx));
    }
    senders
}

fn create_receivers<R>(
    rx: Receiver<AnyPayload>,
    receivers_no: usize,
    create_receiver: R,
    dctx: DistCtx,
) -> Vec<JoinHandle<(usize, Histogram<u64>)>>
where
    R: Fn(Receiver<AnyPayload>, DistCtx) -> JoinHandle<(usize, Histogram<u64>)>,
{
    let mut receivers = Vec::with_capacity(receivers_no);
    for _ in 0..receivers_no {
        receivers.push(create_receiver(rx.clone(), dctx));
    }
    receivers
}

fn create_sync_sender(
    tx: Sender<AnyPayload>,
    n: usize,
    message_size: usize,
    payload: PayloadKind,
    dctx: DistCtx,
) -> JoinHandle<usize> {
    JoinHandle::Sync(thread::spawn(move || {
        let mut error_count = 0;
        let mut sent_count = 0;
        let mut cache = None;
        for seq in 0..n {
            match send(&tx, AnyPayload::fresh(payload, message_size, &mut cache, seq as u64, dctx)) {
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
    dctx: DistCtx,
) -> JoinHandle<usize> {
    JoinHandle::Async(tokio::spawn(async move {
        let mut error_count = 0;
        let mut sent_count = 0;
        let mut cache = None;
        for seq in 0..n {
            match send_async(&tx, AnyPayload::fresh(payload, message_size, &mut cache, seq as u64, dctx)).await {
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

fn create_sync_receiver(rx: Receiver<AnyPayload>, dctx: DistCtx) -> JoinHandle<(usize, Histogram<u64>)> {
    JoinHandle::Sync(thread::spawn(move || {
        let mut received_count = 0;
        let mut hist = Histogram::<u64>::new(3).expect("3 significant digits always valid");
        while let Ok(payload) = recv(&rx) {
            received_count += 1;
            record_stamp(&mut hist, &dctx, &payload);
        }
        (received_count, hist)
    }))
}

fn create_async_receiver(rx: Receiver<AnyPayload>, dctx: DistCtx) -> JoinHandle<(usize, Histogram<u64>)> {
    JoinHandle::Async(tokio::spawn(async move {
        let mut received_count = 0;
        let mut hist = Histogram::<u64>::new(3).expect("3 significant digits always valid");
        while let Ok(payload) = recv_async(&rx).await {
            received_count += 1;
            record_stamp(&mut hist, &dctx, &payload);
        }
        (received_count, hist)
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

pub struct LooleChannel;

impl LatencyChannel<Message> for LooleChannel {
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
    <LooleChannel as LatencyChannel<Message>>::validate(combo, cap)
}

pub fn setup_latency(
    rt: &Runtime,
    combo: Combo,
    cap: Option<usize>,
    message_size: usize,
    warmup: usize,
) -> Result<LatencyProbe, BenchError> {
    setup_ping_pong::<LooleChannel>(rt, combo, cap, message_size, warmup)
}

pub fn bench_latency(
    rt: &Runtime,
    combo: Combo,
    cap: Option<usize>,
    message_size: usize,
) -> Result<LatencyResult, BenchError> {
    ping_pong_latency::<LooleChannel>(rt, combo, cap, message_size)
}
