use std::fmt::Display;

use crate::{
    async_channel_bench,
    bench_utils::{BenchError, BenchResult},
    crossbeam_channel_bench, flume_bench, kanal_bench, loole_bench, message_type, tokio_bench,
};

pub const MESSAGES_NO: usize = 1_000_000;
const MESSAGE_SIZE: usize = 256;

type MsgType = message_type::StackType<MESSAGE_SIZE>;

pub const BUFFER_SIZE_LIST: [Option<usize>; 4] = [Some(0), Some(1), Some(50), Some(100)];

pub enum CrateName {
    Loole,
    Flume,
    AsyncChannel,
    Tokio,
    CrossbeamChannel,
    Kanal,
}

impl Display for CrateName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CrateName::Loole => write!(f, "loole"),
            CrateName::Flume => write!(f, "flume"),
            CrateName::AsyncChannel => write!(f, "async-channel"),
            CrateName::Tokio => write!(f, "tokio"),
            CrateName::CrossbeamChannel => write!(f, "crossbeam-channel"),
            CrateName::Kanal => write!(f, "kanal (std-mutex)"),
        }
    }
}

pub async fn bench_sync_sync(
    crate_name: CrateName,
    senders_no: usize,
    receivers_no: usize,
    buffer_size: Option<usize>,
) -> Result<BenchResult, BenchError> {
    let msg_no = MESSAGES_NO;
    match crate_name {
        CrateName::Loole => {
            loole_bench::bench_sync_sync::<MsgType>(senders_no, receivers_no, buffer_size, msg_no)
                .await
        }
        CrateName::Flume => {
            flume_bench::bench_sync_sync::<MsgType>(senders_no, receivers_no, buffer_size, msg_no)
                .await
        }
        CrateName::AsyncChannel => {
            async_channel_bench::bench_sync_sync::<MsgType>(
                senders_no,
                receivers_no,
                buffer_size,
                msg_no,
            )
            .await
        }
        CrateName::Tokio => {
            tokio_bench::bench_sync_sync::<MsgType>(senders_no, receivers_no, buffer_size, msg_no)
                .await
        }
        CrateName::Kanal => {
            kanal_bench::bench_sync_sync::<MsgType>(senders_no, receivers_no, buffer_size, msg_no)
                .await
        }
        CrateName::CrossbeamChannel => {
            crossbeam_channel_bench::bench_sync_sync::<MsgType>(
                senders_no,
                receivers_no,
                buffer_size,
                msg_no,
            )
            .await
        }
    }
}

pub async fn bench_async_async(
    crate_name: CrateName,
    senders_no: usize,
    receivers_no: usize,
    buffer_size: Option<usize>,
) -> Result<BenchResult, BenchError> {
    let msg_no = MESSAGES_NO;
    match crate_name {
        CrateName::Loole => {
            loole_bench::bench_async_async::<MsgType>(senders_no, receivers_no, buffer_size, msg_no)
                .await
        }
        CrateName::Flume => {
            flume_bench::bench_async_async::<MsgType>(senders_no, receivers_no, buffer_size, msg_no)
                .await
        }
        CrateName::AsyncChannel => {
            async_channel_bench::bench_async_async::<MsgType>(
                senders_no,
                receivers_no,
                buffer_size,
                msg_no,
            )
            .await
        }
        CrateName::Tokio => {
            tokio_bench::bench_async_async::<MsgType>(senders_no, receivers_no, buffer_size, msg_no)
                .await
        }
        CrateName::Kanal => {
            kanal_bench::bench_async_async::<MsgType>(senders_no, receivers_no, buffer_size, msg_no)
                .await
        }
        CrateName::CrossbeamChannel => Err(BenchError::AsyncNotSupported),
    }
}

pub async fn bench_async_sync(
    crate_name: CrateName,
    senders_no: usize,
    receivers_no: usize,
    buffer_size: Option<usize>,
) -> Result<BenchResult, BenchError> {
    let msg_no = MESSAGES_NO;
    match crate_name {
        CrateName::Loole => {
            loole_bench::bench_async_sync::<MsgType>(senders_no, receivers_no, buffer_size, msg_no)
                .await
        }
        CrateName::Flume => {
            flume_bench::bench_async_sync::<MsgType>(senders_no, receivers_no, buffer_size, msg_no)
                .await
        }
        CrateName::AsyncChannel => {
            async_channel_bench::bench_async_sync::<MsgType>(
                senders_no,
                receivers_no,
                buffer_size,
                msg_no,
            )
            .await
        }
        CrateName::Tokio => {
            tokio_bench::bench_async_sync::<MsgType>(senders_no, receivers_no, buffer_size, msg_no)
                .await
        }
        CrateName::Kanal => {
            kanal_bench::bench_async_sync::<MsgType>(senders_no, receivers_no, buffer_size, msg_no)
                .await
        }
        CrateName::CrossbeamChannel => Err(BenchError::AsyncNotSupported),
    }
}

pub async fn bench_sync_async(
    crate_name: CrateName,
    senders_no: usize,
    receivers_no: usize,
    buffer_size: Option<usize>,
) -> Result<BenchResult, BenchError> {
    let msg_no = MESSAGES_NO;
    match crate_name {
        CrateName::Loole => {
            loole_bench::bench_sync_async::<MsgType>(senders_no, receivers_no, buffer_size, msg_no)
                .await
        }
        CrateName::Flume => {
            flume_bench::bench_sync_async::<MsgType>(senders_no, receivers_no, buffer_size, msg_no)
                .await
        }
        CrateName::AsyncChannel => {
            async_channel_bench::bench_sync_async::<MsgType>(
                senders_no,
                receivers_no,
                buffer_size,
                msg_no,
            )
            .await
        }
        CrateName::Tokio => {
            tokio_bench::bench_sync_async::<MsgType>(senders_no, receivers_no, buffer_size, msg_no)
                .await
        }
        CrateName::Kanal => {
            kanal_bench::bench_sync_async::<MsgType>(senders_no, receivers_no, buffer_size, msg_no)
                .await
        }
        CrateName::CrossbeamChannel => Err(BenchError::AsyncNotSupported),
    }
}
