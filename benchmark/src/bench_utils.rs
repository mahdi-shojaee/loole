// The latency trait methods must return a `Send` future so the runner can
// `tokio::spawn` them; `async fn` in a trait cannot express that bound.
#![allow(clippy::manual_async_fn)]

use std::{error::Error, fmt::Display, future::Future, thread, time::Instant};

use hdrhistogram::Histogram;
use tokio::runtime::Runtime;

use crate::config::{DEFAULT_LATENCY_ROUNDS, DEFAULT_LATENCY_WARMUP};
use crate::message_type::{AnyPayload, DistCtx, Message};

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

/// Sender- and receiver-side API flavors of one benchmark scenario: the first
/// word is the flavor the *sender* role uses, the second word the flavor of the
/// *receiver* role (the echoer, for ping-pong latency).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Combo {
    SyncSync,
    AsyncAsync,
    AsyncSync,
    SyncAsync,
}

impl Combo {
    pub fn sender_is_async(self) -> bool {
        matches!(self, Combo::AsyncAsync | Combo::AsyncSync)
    }

    pub fn receiver_is_async(self) -> bool {
        matches!(self, Combo::AsyncAsync | Combo::SyncAsync)
    }
}

/// A spawned-but-not-yet-joined throughput run. Splitting spawn from join lets
/// criterion measure only the transfer itself (`iter_batched`: spawn is setup,
/// join is the timed iteration) — the same `t0`-after-spawn semantics the old
/// hand-rolled timer had.
pub struct ThroughputRun {
    pub senders: Vec<JoinHandle<usize>>,
    /// Receivers return their count plus their local latency histogram: one
    /// histogram per task keeps recording contention-free; callers merge
    /// outside the timed region.
    pub receivers: Vec<JoinHandle<(usize, Histogram<u64>)>>,
}

impl ThroughputRun {
    /// Joins senders then receivers and asserts every message was received.
    /// Returns the counts plus one latency histogram per receiver task.
    /// Callers merge histograms *outside* the timed region (merging inside
    /// would bill microseconds to every iteration): histograms are
    /// associative under merge, so per-iteration merges and one final merge
    /// give identical distributions.
    pub async fn join_all(self) -> (usize, usize, Vec<Histogram<u64>>) {
        let mut sent = 0;
        for s in self.senders {
            sent += s.join().await;
        }
        let mut received = 0;
        let mut hists = Vec::with_capacity(self.receivers.len());
        for r in self.receivers {
            let (n, hist) = r.join().await;
            received += n;
            hists.push(hist);
        }
        assert_eq!(sent, received);
        (sent, received, hists)
    }
}

/// Number of timed round trips per latency measurement.
pub const LATENCY_NO: usize = DEFAULT_LATENCY_ROUNDS;

/// Number of untimed warm-up round trips run before timing.
pub const LATENCY_WARMUP: usize = DEFAULT_LATENCY_WARMUP;

/// Average SPSC ping-pong round-trip latency, in nanoseconds.
pub struct LatencyResult {
    pub latency_ns: usize,
}

/// The four channel handles a ping-pong measurement needs.
///
/// Each role owns two handles: the *sender* role uses `sender_tx` to send and
/// `sender_rx` to receive the echo; the *echoer* role uses `echoer_rx` to
/// receive and `echoer_tx` to send back. Handles are wrapped in the crate's
/// own channel type (e.g. kanal's async ends differ from its sync ends), so
/// `create` decides per combo which variant each handle carries.
pub struct LatencyChannels<C, T>
where
    C: LatencyChannel<T>,
    T: Send + Sync + 'static,
{
    pub sender_tx: C::Sender,
    pub sender_rx: C::Receiver,
    pub echoer_tx: C::Sender,
    pub echoer_rx: C::Receiver,
}

/// A crate's channel API, normalized for the SPSC ping-pong latency benchmark.
pub trait LatencyChannel<T>
where
    T: Send + Sync + 'static,
{
    type Sender: Send + Sync + 'static;
    type Receiver: Send + 'static;

    /// Reject combinations this crate cannot measure (e.g. crossbeam async).
    fn validate(combo: Combo, cap: Option<usize>) -> Result<(), BenchError>;

    /// Create the forward/back channel pair with per-role handles set up for `combo`.
    fn create(combo: Combo, cap: Option<usize>) -> LatencyChannels<Self, T>
    where
        Self: Sized;

    fn send(tx: &Self::Sender, msg: T);

    fn recv(rx: &mut Self::Receiver) -> Option<T>;

    fn send_async(tx: &Self::Sender, msg: T) -> impl Future<Output = ()> + Send + '_;

    fn recv_async(rx: &mut Self::Receiver) -> impl Future<Output = Option<T>> + Send + '_;
}

/// Records one received message's queueing delay into a per-task histogram,
/// if the message carries a send stamp: the stamp counts nanos since the
/// cell's `t0`, so the delay is now minus the stamp. Untracked messages
/// (zero prefix, short payloads, shared mode) record nothing — the common
/// path is a single 8-byte compare on an already-hot cache line, and only
/// sampled messages pay a clock read.
pub fn record_stamp(hist: &mut Histogram<u64>, dctx: &DistCtx, payload: &AnyPayload) {
    if let Some(stamp) = payload.send_stamp_nanos() {
        let now = dctx.t0.elapsed().as_nanos() as u64;
        // `stamp` is send_nanos + 1 (0 = untracked sentinel, filtered above).
        let delta = now.saturating_sub(stamp - 1);
        // Fails only above the maximum trackable value (u64::MAX): impossible.
        let _ = hist.record(delta);
    }
}

/// A prepared ping-pong measurement: echoer live, warm-up done. Running the
/// timed round trips is `probe.run(rt, n)` — criterion's `iter_batched` times
/// exactly that call, excluding channel setup and warm-up.
pub struct LatencyProbe {
    rounds_fn: Box<dyn FnMut(&Runtime, usize) + Send>,
    // Kept so the echoer stays alive for the probe's lifetime; dropping the
    // probe closes the forward channel, which ends the echoer loop.
    _echoer: JoinHandle<()>,
}

impl LatencyProbe {
    /// Runs `rounds` round trips on the caller thread.
    pub fn run(&mut self, rt: &Runtime, rounds: usize) {
        (self.rounds_fn)(rt, rounds)
    }
}

/// Creates the channel pair, spawns the echoer, and runs `warmup` untimed
/// round trips. The returned probe must be dropped to close the forward
/// channel and terminate the echoer.
pub fn setup_ping_pong<C: LatencyChannel<Message>>(
    rt: &Runtime,
    combo: Combo,
    cap: Option<usize>,
    message_size: usize,
    warmup: usize,
) -> Result<LatencyProbe, BenchError> {
    // tokio::spawn in spawn_echoer needs the runtime in scope when called
    // from plain (non-async) setup code.
    let _guard = rt.enter();
    C::validate(combo, cap)?;
    let LatencyChannels {
        sender_tx,
        mut sender_rx,
        echoer_tx,
        mut echoer_rx,
    } = C::create(combo, cap);

    let echoer = if combo.receiver_is_async() {
        JoinHandle::Async(tokio::spawn(async move {
            while let Some(msg) = C::recv_async(&mut echoer_rx).await {
                C::send_async(&echoer_tx, msg).await;
            }
        }))
    } else {
        JoinHandle::Sync(thread::spawn(move || {
            while let Some(msg) = C::recv(&mut echoer_rx) {
                C::send(&echoer_tx, msg);
            }
        }))
    };

    run_rounds::<C>(rt, combo, &sender_tx, &mut sender_rx, message_size, warmup);

    let rounds_fn = Box::new(move |rt: &Runtime, n: usize| {
        run_rounds::<C>(rt, combo, &sender_tx, &mut sender_rx, message_size, n);
    });
    Ok(LatencyProbe {
        rounds_fn,
        _echoer: echoer,
    })
}

fn run_rounds<C: LatencyChannel<Message>>(
    rt: &Runtime,
    combo: Combo,
    tx: &C::Sender,
    rx: &mut C::Receiver,
    message_size: usize,
    n: usize,
) {
    // The latency suite runs in touched mode only (shared runs skip it, and
    // touched is the default): every round trip pings realistic content.
    let message = || Message::new(message_size);
    if combo.sender_is_async() {
        rt.block_on(async {
            for _ in 0..n {
                C::send_async(tx, message()).await;
                C::recv_async(rx).await;
            }
        });
    } else {
        for _ in 0..n {
            C::send(tx, message());
            C::recv(rx);
        }
    }
}

/// Compat helper with the pre-criterion semantics: sets up a probe, times
/// `rounds` round trips, returns the average in nanoseconds.
pub fn ping_pong_latency_n<C: LatencyChannel<Message>>(
    rt: &Runtime,
    combo: Combo,
    cap: Option<usize>,
    message_size: usize,
    warmup: usize,
    rounds: usize,
) -> Result<LatencyResult, BenchError> {
    let mut probe = setup_ping_pong::<C>(rt, combo, cap, message_size, warmup)?;
    let start = Instant::now();
    probe.run(rt, rounds);
    Ok(LatencyResult {
        latency_ns: (start.elapsed().as_nanos() / rounds as u128) as usize,
    })
}

/// Compat helper with default warm-up/round-trip counts.
pub fn ping_pong_latency<C: LatencyChannel<Message>>(
    rt: &Runtime,
    combo: Combo,
    cap: Option<usize>,
    message_size: usize,
) -> Result<LatencyResult, BenchError> {
    ping_pong_latency_n::<C>(rt, combo, cap, message_size, LATENCY_WARMUP, LATENCY_NO)
}

#[cfg(test)]
mod tests {
    use super::{
        ping_pong_latency, setup_ping_pong, BenchError, Combo, LatencyChannel, LatencyChannels,
    };
    use crate::message_type::Message;
    use std::future::Future;
    use std::time::Instant;

    struct TestChannel;

    impl LatencyChannel<Message> for TestChannel {
        type Sender = flume::Sender<Message>;
        type Receiver = flume::Receiver<Message>;

        fn validate(_combo: Combo, _cap: Option<usize>) -> Result<(), BenchError> {
            Ok(())
        }

        fn create(_combo: Combo, cap: Option<usize>) -> LatencyChannels<Self, Message> {
            let pair = || cap.map_or_else(flume::unbounded::<Message>, flume::bounded);
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

        fn recv_async(
            rx: &mut Self::Receiver,
        ) -> impl Future<Output = Option<Message>> + Send + '_ {
            async move { rx.recv_async().await.ok() }
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn latency(combo: Combo) -> Option<usize> {
        let rt = runtime();
        ping_pong_latency::<TestChannel>(&rt, combo, Some(10), 8)
            .ok()
            .map(|result| result.latency_ns)
    }

    #[test]
    fn latency_sync_sync_returns_nonzero() {
        assert!(latency(Combo::SyncSync).unwrap() > 0);
    }

    #[test]
    fn latency_async_async_returns_nonzero() {
        assert!(latency(Combo::AsyncAsync).unwrap() > 0);
    }

    #[test]
    fn latency_async_sync_returns_nonzero() {
        assert!(latency(Combo::AsyncSync).unwrap() > 0);
    }

    #[test]
    fn latency_sync_async_returns_nonzero() {
        assert!(latency(Combo::SyncAsync).unwrap() > 0);
    }

    #[test]
    fn probe_split_setup_run_measures_nonzero_time() {
        let rt = runtime();
        let mut probe =
            setup_ping_pong::<TestChannel>(&rt, Combo::SyncSync, Some(10), 8, 100).unwrap();
        let start = Instant::now();
        probe.run(&rt, 1_000);
        assert!(start.elapsed().as_nanos() > 0);
        // Probe drop closes the forward channel; the echoer must terminate.
        drop(probe);
    }

    #[test]
    fn probe_honors_combo_validation() {
        struct NoAsync;
        impl LatencyChannel<Message> for NoAsync {
            type Sender = flume::Sender<Message>;
            type Receiver = flume::Receiver<Message>;

            fn validate(combo: Combo, _cap: Option<usize>) -> Result<(), BenchError> {
                match combo {
                    Combo::SyncSync => Ok(()),
                    _ => Err(BenchError::AsyncNotSupported),
                }
            }

            fn create(_combo: Combo, cap: Option<usize>) -> LatencyChannels<Self, Message> {
                let pair = || cap.map_or_else(flume::unbounded::<Message>, flume::bounded);
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

            fn send_async(
                _tx: &Self::Sender,
                _msg: Message,
            ) -> impl Future<Output = ()> + Send + '_ {
                async move { unreachable!("no async") }
            }

            fn recv_async(
                _rx: &mut Self::Receiver,
            ) -> impl Future<Output = Option<Message>> + Send + '_ {
                async move { unreachable!("no async") }
            }
        }

        let rt = runtime();
        assert!(matches!(
            setup_ping_pong::<NoAsync>(&rt, Combo::AsyncAsync, Some(10), 8, 10),
            Err(BenchError::AsyncNotSupported)
        ));
        assert!(setup_ping_pong::<NoAsync>(&rt, Combo::SyncSync, Some(10), 8, 10).is_ok());
    }

    #[test]
    fn crossbeam_async_combos_rejected() {
        let rt = runtime();
        for combo in [Combo::AsyncAsync, Combo::AsyncSync, Combo::SyncAsync] {
            let res = crate::benchmark::bench_latency(
                &rt,
                crate::benchmark::CrateName::CrossbeamChannel,
                combo,
                Some(1),
                8,
            );
            assert!(matches!(res, Err(BenchError::AsyncNotSupported)));
        }
    }

    #[test]
    fn tokio_zero_capacity_rejected() {
        let rt = runtime();
        let res = crate::benchmark::bench_latency(
            &rt,
            crate::benchmark::CrateName::Tokio,
            Combo::SyncSync,
            Some(0),
            8,
        );
        assert!(matches!(res, Err(BenchError::ZeroCapacityNotSupported)));
    }
}
