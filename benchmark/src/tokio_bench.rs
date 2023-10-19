use std::{future::Future, thread};
use tokio::sync::mpsc::{
    self, channel, error::SendError, unbounded_channel, Receiver, Sender, UnboundedReceiver,
    UnboundedSender,
};

use crate::bench_utils::{calculate_benchmark_result, BenchError, BenchResult, JoinHandle};

const UNBOUNDED_BUFFER_SIZE_ESTIMATION: usize = 100_000;

pub fn crate_name() -> &'static str {
    "loole"
}

pub fn send<T>(tx: &Sender<T>, msg: T) -> Result<(), SendError<T>> {
    tx.blocking_send(msg)
}

pub fn recv<T>(rx: &mut Receiver<T>) -> Option<T> {
    rx.blocking_recv()
}

pub fn send_async<T: 'static>(
    tx: &Sender<T>,
    msg: T,
) -> impl Future<Output = Result<(), SendError<T>>> + '_ {
    tx.send(msg)
}

pub fn recv_async<T>(rx: &mut Receiver<T>) -> impl Future<Output = Option<T>> + '_ {
    rx.recv()
}

async fn bench_helper<T, S, R>(
    senders_no: usize,
    receivers_no: usize,
    cap: Option<usize>,
    msg_no: usize,
    create_sender: S,
    create_receiver: R,
) -> Result<BenchResult, BenchError>
where
    T: From<usize> + Send + 'static,
    S: Fn(Sender<T>, usize) -> JoinHandle<usize>,
    R: Fn(Receiver<T>) -> JoinHandle<usize>,
{
    if receivers_no > 1 {
        return Err(BenchError::MpmcNotSupported);
    }
    if cap == Some(0) {
        return Err(BenchError::ZeroCapacityNotSupported);
    }
    let (tx, rx) = channel(cap.unwrap_or(UNBOUNDED_BUFFER_SIZE_ESTIMATION));
    let senders = create_senders(tx, senders_no, msg_no, create_sender);
    let receivers = create_receivers(rx, receivers_no, create_receiver);
    Ok(calculate_benchmark_result(senders, receivers).await)
}

fn create_senders<T, S>(
    tx: Sender<T>,
    senders_no: usize,
    msg_no: usize,
    create_sender: S,
) -> Vec<JoinHandle<usize>>
where
    S: Fn(Sender<T>, usize) -> JoinHandle<usize>,
{
    let mut senders = Vec::with_capacity(senders_no);
    for i in 0..senders_no {
        let n = msg_no / senders_no + if i < msg_no % senders_no { 1 } else { 0 };
        senders.push(create_sender(tx.clone(), n));
    }
    senders
}

fn create_receivers<T, R>(
    rx: Receiver<T>,
    receivers_no: usize,
    create_receiver: R,
) -> Vec<JoinHandle<usize>>
where
    R: Fn(Receiver<T>) -> JoinHandle<usize>,
{
    let mut receivers = Vec::with_capacity(receivers_no);
    receivers.push(create_receiver(rx));
    receivers
}

fn create_sync_sender<T>(tx: Sender<T>, n: usize) -> JoinHandle<usize>
where
    T: From<usize> + Send + 'static,
{
    JoinHandle::Sync(thread::spawn(move || {
        for k in 0..n {
            match send(&tx, k.into()) {
                Ok(_) => (),
                Err(_) => println!("error: channel closed at: {}", k),
            }
        }
        n
    }))
}

fn create_async_sender<T>(tx: Sender<T>, n: usize) -> JoinHandle<usize>
where
    T: From<usize> + Send + 'static,
{
    JoinHandle::Async(tokio::spawn(async move {
        for k in 0..n {
            match send_async(&tx, k.into()).await {
                Ok(_) => (),
                Err(_) => println!("error: channel closed at: {}", k),
            }
        }
        n
    }))
}

fn create_sync_receiver<T>(mut rx: Receiver<T>) -> JoinHandle<usize>
where
    T: From<usize> + Send + 'static,
{
    JoinHandle::Sync(thread::spawn(move || {
        let mut c = 0;
        loop {
            match recv(&mut rx) {
                Some(_) => c += 1,
                None => break c,
            }
        }
    }))
}

fn create_async_receiver<T>(mut rx: Receiver<T>) -> JoinHandle<usize>
where
    T: From<usize> + Send + 'static,
{
    JoinHandle::Async(tokio::spawn(async move {
        let mut c = 0;
        loop {
            match recv_async(&mut rx).await {
                Some(_) => c += 1,
                None => break c,
            }
        }
    }))
}

pub async fn bench_sync_sync<T>(
    senders_no: usize,
    receivers_no: usize,
    buffer_size: Option<usize>,
    msg_no: usize,
) -> Result<BenchResult, BenchError>
where
    T: From<usize> + Send + 'static,
{
    bench_helper(
        senders_no,
        receivers_no,
        buffer_size,
        msg_no,
        create_sync_sender::<T>,
        create_sync_receiver::<T>,
    )
    .await
}

pub async fn bench_async_async<T>(
    senders_no: usize,
    receivers_no: usize,
    buffer_size: Option<usize>,
    msg_no: usize,
) -> Result<BenchResult, BenchError>
where
    T: From<usize> + Send + Sync + 'static,
{
    bench_helper(
        senders_no,
        receivers_no,
        buffer_size,
        msg_no,
        create_async_sender::<T>,
        create_async_receiver::<T>,
    )
    .await
}

pub async fn bench_async_sync<T>(
    senders_no: usize,
    receivers_no: usize,
    buffer_size: Option<usize>,
    msg_no: usize,
) -> Result<BenchResult, BenchError>
where
    T: From<usize> + Send + Sync + 'static,
{
    bench_helper(
        senders_no,
        receivers_no,
        buffer_size,
        msg_no,
        create_async_sender::<T>,
        create_sync_receiver::<T>,
    )
    .await
}

pub async fn bench_sync_async<T>(
    senders_no: usize,
    receivers_no: usize,
    buffer_size: Option<usize>,
    msg_no: usize,
) -> Result<BenchResult, BenchError>
where
    T: From<usize> + Send + Sync + 'static,
{
    bench_helper(
        senders_no,
        receivers_no,
        buffer_size,
        msg_no,
        create_sync_sender::<T>,
        create_async_receiver::<T>,
    )
    .await
}
