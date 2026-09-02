use std::fmt::Display;
use std::str::FromStr;

use tokio::runtime::Runtime;

use crate::{
    async_channel_bench,
    bench_utils::{BenchError, Combo, LatencyProbe, LatencyResult, ThroughputRun},
    crossbeam_channel_bench, flume_bench, kanal_bench, loole_bench, tokio_bench,
};

/// All benchmarkable crates in their canonical (column) order.
pub const ALL_CRATES: [CrateName; 6] = [
    CrateName::Loole,
    CrateName::Flume,
    CrateName::AsyncChannel,
    CrateName::Tokio,
    CrateName::Kanal,
    CrateName::CrossbeamChannel,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

impl FromStr for CrateName {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "loole" => Ok(CrateName::Loole),
            "flume" => Ok(CrateName::Flume),
            "async-channel" => Ok(CrateName::AsyncChannel),
            "tokio" => Ok(CrateName::Tokio),
            // Alias: the canonical name contains a space and is awkward to pass.
            "kanal" | "kanal (std-mutex)" => Ok(CrateName::Kanal),
            "crossbeam-channel" => Ok(CrateName::CrossbeamChannel),
            other => Err(format!(
                "unknown crate '{other}' (valid: {})",
                ALL_CRATES
                    .iter()
                    .map(|c| format!("\"{c}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }
}

/// Whether the (crate, combo, receivers, capacity) throughput cell exists.
///
/// Mirrors the adapters' error paths without running anything; the harness
/// registers criterion benchmarks only for `true` cells and the chart
/// converter renders missing cells as 0 — like the old runner's `Err => 0`.
pub fn supports_throughput(
    krate: CrateName,
    combo: Combo,
    receivers_no: usize,
    cap: Option<usize>,
) -> bool {
    match krate {
        CrateName::CrossbeamChannel => matches!(combo, Combo::SyncSync),
        CrateName::Tokio => receivers_no <= 1 && cap != Some(0),
        CrateName::AsyncChannel => cap != Some(0),
        _ => true,
    }
}

/// Whether the (crate, combo, capacity) latency cell exists.
pub fn supports_latency(krate: CrateName, combo: Combo, cap: Option<usize>) -> bool {
    validate_latency(krate, combo, cap).is_ok()
}

fn validate_latency(krate: CrateName, combo: Combo, cap: Option<usize>) -> Result<(), BenchError> {
    match krate {
        CrateName::Loole => loole_bench::validate_latency(combo, cap),
        CrateName::Flume => flume_bench::validate_latency(combo, cap),
        CrateName::AsyncChannel => async_channel_bench::validate_latency(combo, cap),
        CrateName::Tokio => tokio_bench::validate_latency(combo, cap),
        CrateName::Kanal => kanal_bench::validate_latency(combo, cap),
        CrateName::CrossbeamChannel => crossbeam_channel_bench::validate_latency(combo, cap),
    }
}

/// Spawns senders and receivers (untimed; not yet joined). Call
/// [`ThroughputRun::join_all`] under the timed region.
///
/// `rt` is only used for its scope: adapters spawn tokio tasks for async
/// roles, which requires a runtime context even outside `block_on`.
pub fn setup_throughput(
    rt: &Runtime,
    krate: CrateName,
    combo: Combo,
    senders_no: usize,
    receivers_no: usize,
    cap: Option<usize>,
    msg_no: usize,
    message_size: usize,
) -> Result<ThroughputRun, BenchError> {
    let _guard = rt.enter();
    macro_rules! per_crate {
        ($module:ident) => {
            match combo {
                Combo::SyncSync => {
                    $module::setup_sync_sync(senders_no, receivers_no, cap, msg_no, message_size)
                }
                Combo::AsyncAsync => {
                    $module::setup_async_async(senders_no, receivers_no, cap, msg_no, message_size)
                }
                Combo::AsyncSync => {
                    $module::setup_async_sync(senders_no, receivers_no, cap, msg_no, message_size)
                }
                Combo::SyncAsync => {
                    $module::setup_sync_async(senders_no, receivers_no, cap, msg_no, message_size)
                }
            }
        };
    }
    match krate {
        CrateName::Loole => per_crate!(loole_bench),
        CrateName::Flume => per_crate!(flume_bench),
        CrateName::AsyncChannel => per_crate!(async_channel_bench),
        CrateName::Tokio => per_crate!(tokio_bench),
        CrateName::Kanal => per_crate!(kanal_bench),
        CrateName::CrossbeamChannel => match combo {
            Combo::SyncSync => crossbeam_channel_bench::setup_sync_sync(
                senders_no,
                receivers_no,
                cap,
                msg_no,
                message_size,
            ),
            _ => Err(BenchError::AsyncNotSupported),
        },
    }
}

/// Prepares a ping-pong latency probe (channels, echoer, warm-up) for a crate.
pub fn setup_latency(
    rt: &Runtime,
    krate: CrateName,
    combo: Combo,
    cap: Option<usize>,
    message_size: usize,
    warmup: usize,
) -> Result<LatencyProbe, BenchError> {
    match krate {
        CrateName::Loole => loole_bench::setup_latency(rt, combo, cap, message_size, warmup),
        CrateName::Flume => flume_bench::setup_latency(rt, combo, cap, message_size, warmup),
        CrateName::AsyncChannel => {
            async_channel_bench::setup_latency(rt, combo, cap, message_size, warmup)
        }
        CrateName::Tokio => tokio_bench::setup_latency(rt, combo, cap, message_size, warmup),
        CrateName::Kanal => kanal_bench::setup_latency(rt, combo, cap, message_size, warmup),
        CrateName::CrossbeamChannel => {
            crossbeam_channel_bench::setup_latency(rt, combo, cap, message_size, warmup)
        }
    }
}

/// Compat helper measuring a full latency run (setup + timed rounds) for tests.
pub fn bench_latency(
    rt: &Runtime,
    krate: CrateName,
    combo: Combo,
    cap: Option<usize>,
    message_size: usize,
) -> Result<LatencyResult, BenchError> {
    match krate {
        CrateName::Loole => loole_bench::bench_latency(rt, combo, cap, message_size),
        CrateName::Flume => flume_bench::bench_latency(rt, combo, cap, message_size),
        CrateName::AsyncChannel => async_channel_bench::bench_latency(rt, combo, cap, message_size),
        CrateName::Tokio => tokio_bench::bench_latency(rt, combo, cap, message_size),
        CrateName::Kanal => kanal_bench::bench_latency(rt, combo, cap, message_size),
        CrateName::CrossbeamChannel => {
            crossbeam_channel_bench::bench_latency(rt, combo, cap, message_size)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    // --- supports_throughput ---

    #[test]
    fn crossbeam_supports_only_sync_sync_throughput() {
        for cap in [Some(0), Some(1), Some(10), Some(50), Some(100), None] {
            assert!(supports_throughput(
                CrateName::CrossbeamChannel,
                Combo::SyncSync,
                10,
                cap
            ));
            for combo in [Combo::AsyncAsync, Combo::AsyncSync, Combo::SyncAsync] {
                assert!(!supports_throughput(
                    CrateName::CrossbeamChannel,
                    combo,
                    10,
                    cap
                ));
            }
        }
    }

    #[test]
    fn tokio_rejects_mpmc_and_zero_capacity() {
        assert!(!supports_throughput(
            CrateName::Tokio,
            Combo::SyncSync,
            10,
            Some(1)
        ));
        assert!(!supports_throughput(
            CrateName::Tokio,
            Combo::SyncSync,
            10,
            None
        ));
        assert!(!supports_throughput(
            CrateName::Tokio,
            Combo::AsyncAsync,
            1,
            Some(0)
        ));
        assert!(supports_throughput(
            CrateName::Tokio,
            Combo::SyncSync,
            1,
            Some(1)
        ));
        assert!(supports_throughput(
            CrateName::Tokio,
            Combo::AsyncAsync,
            1,
            None
        ));
    }

    #[test]
    fn async_channel_rejects_zero_capacity() {
        assert!(!supports_throughput(
            CrateName::AsyncChannel,
            Combo::SyncSync,
            10,
            Some(0)
        ));
        assert!(supports_throughput(
            CrateName::AsyncChannel,
            Combo::SyncSync,
            10,
            Some(1)
        ));
        assert!(supports_throughput(
            CrateName::AsyncChannel,
            Combo::AsyncAsync,
            10,
            None
        ));
    }

    #[test]
    fn loole_flume_kanal_support_everything() {
        for krate in [CrateName::Loole, CrateName::Flume, CrateName::Kanal] {
            for (combo, token) in crate::ids::COMBOS {
                for cap in [Some(0), Some(1), Some(10), Some(50), Some(100), None] {
                    assert!(
                        supports_throughput(krate, combo, 10, cap),
                        "{krate} {token} {cap:?}"
                    );
                    assert!(supports_latency(krate, combo, cap));
                }
            }
        }
    }

    // --- supports_latency ---

    #[test]
    fn latency_support_matrix() {
        for combo in [Combo::AsyncAsync, Combo::AsyncSync, Combo::SyncAsync] {
            assert!(!supports_latency(
                CrateName::CrossbeamChannel,
                combo,
                Some(1)
            ));
        }
        assert!(supports_latency(
            CrateName::CrossbeamChannel,
            Combo::SyncSync,
            Some(0)
        ));
        assert!(!supports_latency(
            CrateName::Tokio,
            Combo::SyncSync,
            Some(0)
        ));
        assert!(
            supports_latency(CrateName::Tokio, Combo::SyncSync, None)
                && supports_latency(CrateName::Tokio, Combo::SyncSync, Some(1))
        );
        assert!(!supports_latency(
            CrateName::AsyncChannel,
            Combo::SyncSync,
            Some(0)
        ));
        assert!(supports_latency(
            CrateName::AsyncChannel,
            Combo::SyncSync,
            Some(1)
        ));
    }

    // --- setup/join split ---

    #[test]
    fn throughput_run_transfers_all_messages_sync_sync() {
        let rt = runtime();
        let run = setup_throughput(
            &rt,
            CrateName::Loole,
            Combo::SyncSync,
            2,
            3,
            Some(10),
            1_000,
            8,
        )
        .unwrap();
        let (sent, received) = rt.block_on(run.join_all());
        assert_eq!(sent, 1_000);
        assert_eq!(received, 1_000);
    }

    #[test]
    fn throughput_run_transfers_all_messages_async_async() {
        let rt = runtime();
        let run = setup_throughput(
            &rt,
            CrateName::Flume,
            Combo::AsyncAsync,
            2,
            3,
            Some(10),
            1_000,
            8,
        )
        .unwrap();
        let (sent, received) = rt.block_on(run.join_all());
        assert_eq!(sent, 1_000);
        assert_eq!(received, 1_000);
    }

    #[test]
    fn throughput_run_transfers_all_messages_mixed_combos() {
        let rt = runtime();
        for combo in [Combo::AsyncSync, Combo::SyncAsync] {
            let run = setup_throughput(&rt, CrateName::Kanal, combo, 2, 2, None, 500, 8).unwrap();
            let (sent, received) = rt.block_on(run.join_all());
            assert_eq!((sent, received), (500, 500), "{combo:?}");
        }
    }

    #[test]
    fn setup_throughput_rejects_unsupported() {
        let rt = runtime();
        assert!(matches!(
            setup_throughput(&rt, CrateName::Tokio, Combo::SyncSync, 2, 2, Some(1), 10, 8),
            Err(BenchError::MpmcNotSupported)
        ));
        assert!(matches!(
            setup_throughput(
                &rt,
                CrateName::AsyncChannel,
                Combo::SyncSync,
                1,
                1,
                Some(0),
                10,
                8
            ),
            Err(BenchError::ZeroCapacityNotSupported)
        ));
        assert!(matches!(
            setup_throughput(
                &rt,
                CrateName::CrossbeamChannel,
                Combo::AsyncAsync,
                1,
                1,
                Some(1),
                10,
                8
            ),
            Err(BenchError::AsyncNotSupported)
        ));
    }

    #[test]
    fn latency_probe_via_crate_dispatch() {
        let rt = runtime();
        // Cross-crate smoke: every supported channel-shape runs at all.
        for (krate, combo, cap) in [
            (CrateName::Loole, Combo::SyncSync, Some(10)),
            (CrateName::Flume, Combo::AsyncAsync, None),
            (CrateName::Tokio, Combo::SyncAsync, Some(10)),
            (CrateName::Kanal, Combo::AsyncSync, Some(10)),
            (CrateName::AsyncChannel, Combo::AsyncAsync, Some(10)),
            (CrateName::CrossbeamChannel, Combo::SyncSync, None),
        ] {
            let mut probe = setup_latency(&rt, krate, combo, cap, 8, 100).unwrap();
            let start = std::time::Instant::now();
            probe.run(&rt, 1_000);
            assert!(start.elapsed().as_nanos() > 0, "{krate} {combo:?} {cap:?}");
        }
    }

    #[test]
    fn bench_latency_compat_returns_nonzero() {
        let rt = runtime();
        let result = bench_latency(&rt, CrateName::Kanal, Combo::AsyncAsync, Some(10), 8).unwrap();
        assert!(result.latency_ns > 0);
    }
}
