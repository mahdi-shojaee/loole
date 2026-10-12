// The latency trait methods must return a `Send` future so the runner can
// `tokio::spawn` them; `async fn` in a trait cannot express that bound.
#![allow(clippy::manual_async_fn)]

use std::{error::Error, fmt::Display, future::Future, thread, time::Instant};

use tokio::runtime::Runtime;

use crate::config::{DEFAULT_LATENCY_ROUNDS, DEFAULT_LATENCY_WARMUP};
use crate::message_type::{Message, PayloadKind};

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
    pub receivers: Vec<JoinHandle<usize>>,
}

impl ThroughputRun {
    /// Joins senders then receivers and asserts every message was received.
    pub async fn join_all(self) -> (usize, usize) {
        let mut sent = 0;
        for s in self.senders {
            sent += s.join().await;
        }
        let mut received = 0;
        for r in self.receivers {
            received += r.join().await;
        }
        assert_eq!(sent, received);
        (sent, received)
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
    payload: PayloadKind,
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

    run_rounds::<C>(rt, combo, &sender_tx, &mut sender_rx, message_size, warmup, payload);

    let rounds_fn = Box::new(move |rt: &Runtime, n: usize| {
        run_rounds::<C>(rt, combo, &sender_tx, &mut sender_rx, message_size, n, payload);
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
    payload: PayloadKind,
) {
    // The echoed content matches the run's payload mode so benchmark IDs
    // stay truthful. `Shared` has no owned-content probe (its content is one
    // cloned Arc, unrepresentable through `LatencyChannel<Message>`); shared
    // runs skip the latency suite, so this arm is a fallback that preserves
    // the historical zeroed-content behavior.
    let message = || match payload {
        PayloadKind::Alloc | PayloadKind::Shared => Message::new(message_size),
        PayloadKind::Touched => Message::new_touched(message_size),
    };
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
    payload: PayloadKind,
) -> Result<LatencyResult, BenchError> {
    let mut probe = setup_ping_pong::<C>(rt, combo, cap, message_size, warmup, payload)?;
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
    payload: PayloadKind,
) -> Result<LatencyResult, BenchError> {
    ping_pong_latency_n::<C>(rt, combo, cap, message_size, LATENCY_WARMUP, LATENCY_NO, payload)
}

#[cfg(test)]
mod tests {
    use super::{
        ping_pong_latency, setup_ping_pong, BenchError, Combo, LatencyChannel, LatencyChannels,
        PayloadKind,
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
        ping_pong_latency::<TestChannel>(&rt, combo, Some(10), 8, PayloadKind::Alloc)
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
            setup_ping_pong::<TestChannel>(&rt, Combo::SyncSync, Some(10), 8, 100, PayloadKind::Alloc).unwrap();
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
            setup_ping_pong::<NoAsync>(&rt, Combo::AsyncAsync, Some(10), 8, 10, PayloadKind::Alloc),
            Err(BenchError::AsyncNotSupported)
        ));
        assert!(setup_ping_pong::<NoAsync>(&rt, Combo::SyncSync, Some(10), 8, 10, PayloadKind::Alloc).is_ok());
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
                PayloadKind::Alloc,
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
            PayloadKind::Alloc,
        );
        assert!(matches!(res, Err(BenchError::ZeroCapacityNotSupported)));
    }
}
