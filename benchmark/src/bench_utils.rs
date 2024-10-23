use std::{error::Error, fmt::Display, thread, time::Instant};

pub struct BenchResult {
    pub throughput: usize,
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug)]
pub enum BenchError {
    ZeroCapacityNotSupported,
    AsyncNotSupported,
    MpmcNotSupported,
}

impl Display for BenchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BenchError::ZeroCapacityNotSupported => write!(f, "zero capacity not supported"),
            BenchError::AsyncNotSupported => write!(f, "async not supported"),
            BenchError::MpmcNotSupported => write!(f, "mpmc not supported"),
        }
    }
}

impl Error for BenchError {}

pub enum JoinHandle<T> {
    Sync(thread::JoinHandle<T>),
    Async(tokio::task::JoinHandle<T>),
}

impl<T> JoinHandle<T> {
    pub async fn join(self) -> T {
        match self {
            JoinHandle::Sync(h) => h.join().unwrap(),
            JoinHandle::Async(h) => h.await.unwrap(),
        }
    }
}

pub async fn calculate_benchmark_result(
    senders: Vec<JoinHandle<usize>>,
    receivers: Vec<JoinHandle<usize>>,
) -> BenchResult {
    let t = Instant::now();
    let mut sent_no = 0;
    for t in senders {
        sent_no += t.join().await;
    }
    let mut recv_no = 0;
    for t in receivers {
        recv_no += t.join().await;
    }
    let elapsed = t.elapsed();
    assert_eq!(sent_no, recv_no);
    let throughput = (recv_no as f32 / elapsed.as_secs_f32()).round() as usize;
    BenchResult { throughput }
}
