//! Regression tests for `SendSink` soundness under concurrent `forward()`.
//!
//! Shapes covered:
//! - ONE input channel whose receiver is CLONED per worker; many tasks call
//!   `recv_async()` on the clones concurrently;
//! - workers forward results via `send_async` into ONE output channel;
//! - the driver collects via `into_stream()` while producers still send
//!   (including `Sink::forward` / `into_sink` on the feed side).
//!
//! Two past defects are pinned here:
//! - `poll_ready` reported always-ready instead of flushing the buffered
//!   send, so `forward()` delivered only the last item;
//! - `poll_close` closed the whole shared channel instead of releasing only
//!   its own sender handle, so concurrent sinks failed with spurious
//!   `SendError` under contention.
//!
//! Single-consumer `send_async` feeds, lone `recv_async` loops, lone
//! `RecvStream` collects, `bounded(1)` ping-pong, and `Stream::for_each` over
//! cloned streams all PASS and are therefore NOT covered here.
//! Isolation result (verified): feeding via plain `send_async` delivers all
//! items — the loss is specific to the `Sink` (`into_sink` + `forward`) path.
//!
//! Bounded waits turn hangs into failures so the suite always terminates.
//!
//! DO NOT "fix" these by weakening assertions — they encode the contract.

use futures::StreamExt as _;
use loole::unbounded;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(30);
const WORKERS: usize = 8;

/// Fan-out shape: feeder `forward`s 0..N into the input channel; WORKERS
/// tasks share `input_rx.clone()` via `recv_async`, map the value, and
/// `send_async` into the output channel; driver collects
/// `output_rx.into_stream()`.
#[test]
fn cloned_receivers_concurrent_recv_async_deliver_all() {
    const N: usize = 4000;

    async_std::task::block_on(async {
        let (input_tx, input_rx) = unbounded::<usize>();
        let (output_tx, output_rx) = unbounded::<usize>();

        // feeder: forward into the input sink
        let feed = async_std::task::spawn(async move {
            futures::stream::iter(0..N)
                .map(Ok)
                .forward(input_tx.into_sink())
                .await
                .unwrap();
        });

        // workers share cloned receivers, one clone per worker
        let consumed = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut workers = Vec::new();
        for _ in 0..WORKERS {
            let input_rx = input_rx.clone();
            let output_tx = output_tx.clone();
            let consumed = consumed.clone();
            workers.push(async_std::task::spawn(async move {
                while let Ok(v) = input_rx.recv_async().await {
                    consumed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    output_tx.send_async(v * 2).await.unwrap();
                }
            }));
        }
        drop(output_tx);

        let collect = async {
            let mut got = Vec::new();
            let mut stream = output_rx.into_stream();
            while let Some(v) = stream.next().await {
                got.push(v);
            }
            got
        };
        let mut got = async_std::future::timeout(TIMEOUT, collect)
            .await
            .expect("timed out: cloned-receiver fan-out hung");
        feed.await;
        for w in workers {
            w.await;
        }

        got.sort_unstable();
        let want: Vec<usize> = (0..N).map(|v| v * 2).collect();
        let consumed = consumed.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            consumed, N,
            "workers' recv_async saw only {consumed} of {N} inputs (input side loses items)"
        );
        assert_eq!(got.len(), want.len(), "lost items in cloned-receiver fan-out");
        assert_eq!(got, want);
    });
}

/// Minimal case: `forward` N items into an `unbounded` sink, then collect.
/// (Feeding the same workload with plain `send_async` in a loop delivers all
/// items — the loss is specific to the Sink path.)
#[test]
fn sink_forward_delivers_every_item() {
    const N: usize = 4000;

    async_std::task::block_on(async {
        let (tx, rx) = unbounded::<usize>();
        futures::stream::iter(0..N)
            .map(Ok)
            .forward(tx.into_sink())
            .await
            .unwrap();

        let collect = async {
            let mut got = Vec::new();
            let mut stream = rx.into_stream();
            while let Some(v) = stream.next().await {
                got.push(v);
            }
            got
        };
        let mut got = async_std::future::timeout(TIMEOUT, collect)
            .await
            .expect("timed out collecting forwarded items");
        got.sort_unstable();
        let want: Vec<usize> = (0..N).collect();
        assert_eq!(got.len(), want.len(), "Sink::forward lost items");
        assert_eq!(got, want);
    });
}

/// Concurrent `into_sink().forward()` tasks over cloned senders into ONE
/// output channel must all succeed while a receiver is alive. Fails with
/// spurious `SendError` (disconnected) under contention.
#[test]
fn concurrent_sinks_forward_all_succeed() {
    const WORKERS: usize = 8;
    const PER_WORKER: usize = 500;

    async_std::task::block_on(async {
        let (tx, rx) = unbounded::<usize>();
        let mut tasks = Vec::new();
        for w in 0..WORKERS {
            let tx = tx.clone();
            tasks.push(async_std::task::spawn(async move {
                futures::stream::iter(0..PER_WORKER)
                    .map(move |i| Ok(w * PER_WORKER + i))
                    .forward(tx.into_sink())
                    .await
                    .unwrap();
            }));
        }
        drop(tx);
        let collect = async {
            let mut got = Vec::new();
            let mut stream = rx.into_stream();
            while let Some(v) = stream.next().await {
                got.push(v);
            }
            got
        };
        let mut got =
            async_std::future::timeout(std::time::Duration::from_secs(30), collect)
                .await
                .expect("timed out");
        for t in tasks {
            t.await;
        }
        got.sort_unstable();
        let want: Vec<usize> = (0..WORKERS * PER_WORKER).collect();
        assert_eq!(got.len(), want.len(), "concurrent sinks lost items");
        assert_eq!(got, want);
    });
}
