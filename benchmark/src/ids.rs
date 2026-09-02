//! Criterion benchmark ID construction.
//!
//! Every registered benchmark gets a self-describing ID so the chart converter
//! (`benchmark/charts/collect-criterion.js`) can reconstruct full scenario metadata
//! from the path criterion writes results to:
//!
//! ```text
//! <metric>-<size>-n<count>/<topology>-s<S>r<R>/<combo>/<cap>/<crate>
//! e.g. throughput-256-n100000/mpsc-s1000r1/sync-sync/b10/loole
//!      latency-32-n100000/spsc-s1r1/async-sync/unbounded/kanal-std-mutex
//! ```
//!
//! Tokens are lowercase/digits/hyphens only (criterion uses IDs as path segments), so
//! the JS side can parse them with plain splits. Keep both sides in sync.

use crate::bench_utils::Combo;
use crate::benchmark::CrateName;

/// Sender/receiver API-flavor combos in fixed order. This order is also the
/// column order of the bounded-capacity datasets' rows.
pub const COMBOS: [(Combo, &str); 4] = [
    (Combo::SyncSync, "sync-sync"),
    (Combo::AsyncAsync, "async-async"),
    (Combo::AsyncSync, "async-sync"),
    (Combo::SyncAsync, "sync-async"),
];

/// Capacities in chart-row order; `None` is the unbounded-channel scenario,
/// rendered as the "unbounded" dataset of each topology.
pub const CAPACITIES: [(Option<usize>, &str); 6] = [
    (Some(0), "b0"),
    (Some(1), "b1"),
    (Some(10), "b10"),
    (Some(50), "b50"),
    (Some(100), "b100"),
    (None, "unbounded"),
];

pub fn combo_token(combo: Combo) -> &'static str {
    COMBOS
        .iter()
        .find(|(c, _)| *c == combo)
        .map(|(_, token)| *token)
        .expect("every Combo has a token")
}

pub fn capacity_token(cap: Option<usize>) -> &'static str {
    CAPACITIES
        .iter()
        .find(|(c, _)| *c == cap)
        .map(|(_, token)| *token)
        .unwrap_or_else(|| panic!("capacity {cap:?} is not in the chart matrix"))
}

/// Crate token used in benchmark IDs. Kanal's display name ("kanal (std-mutex)")
/// contains characters we don't want in paths, hence the mangled token.
pub fn crate_token(krate: CrateName) -> String {
    match krate {
        CrateName::Kanal => "kanal-std-mutex".to_string(),
        other => other.to_string(),
    }
}

/// One of the two measured quantities; the `metric` segment of an ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricKind {
    Throughput,
    Latency,
}

impl MetricKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MetricKind::Throughput => "throughput",
            MetricKind::Latency => "latency",
        }
    }
}

/// Sender/receiver topology of a throughput benchmark. Latency is always SPSC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Topology {
    Spsc,
    Mpsc,
    Mpmc,
}

/// e.g. `spsc-s1r1`, `mpsc-s1000r1`, `mpmc-s1000r10`.
pub fn topology_token(topology: Topology, senders: usize, receivers: usize) -> String {
    let name = match topology {
        Topology::Spsc => "spsc",
        Topology::Mpsc => "mpsc",
        Topology::Mpmc => "mpmc",
    };
    format!("{name}-s{senders}r{receivers}")
}

/// The criterion *group* ID (workload-level): `<metric>-<size>-n<count>`.
/// `count` is messages-per-iteration for throughput, round-trips for latency.
pub fn group_id(kind: MetricKind, message_size: usize, workload: usize) -> String {
    format!("{}-{message_size}-n{workload}", kind.as_str())
}

/// The criterion *function* ID within a group: `<combo>/<cap>/<crate>`.
pub fn bench_id(combo: Combo, cap: Option<usize>, krate: CrateName) -> String {
    format!(
        "{}/{}/{}",
        combo_token(combo),
        capacity_token(cap),
        crate_token(krate)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_ids_encode_metric_size_and_workload() {
        assert_eq!(
            group_id(MetricKind::Throughput, 256, 100_000),
            "throughput-256-n100000"
        );
        assert_eq!(
            group_id(MetricKind::Latency, 32, 50_000),
            "latency-32-n50000"
        );
    }

    #[test]
    fn metric_kind_tokens() {
        assert_eq!(MetricKind::Throughput.as_str(), "throughput");
        assert_eq!(MetricKind::Latency.as_str(), "latency");
    }

    #[test]
    fn topology_tokens_embed_counts() {
        assert_eq!(topology_token(Topology::Spsc, 1, 1), "spsc-s1r1");
        assert_eq!(topology_token(Topology::Mpsc, 1000, 1), "mpsc-s1000r1");
        assert_eq!(topology_token(Topology::Mpmc, 8, 4), "mpmc-s8r4");
    }

    #[test]
    fn combo_tokens_match_chart_dataset_names() {
        assert_eq!(combo_token(Combo::SyncSync), "sync-sync");
        assert_eq!(combo_token(Combo::AsyncAsync), "async-async");
        assert_eq!(combo_token(Combo::AsyncSync), "async-sync");
        assert_eq!(combo_token(Combo::SyncAsync), "sync-async");
    }

    #[test]
    fn capacity_tokens() {
        assert_eq!(capacity_token(Some(0)), "b0");
        assert_eq!(capacity_token(Some(1)), "b1");
        assert_eq!(capacity_token(Some(10)), "b10");
        assert_eq!(capacity_token(Some(50)), "b50");
        assert_eq!(capacity_token(Some(100)), "b100");
        assert_eq!(capacity_token(None), "unbounded");
    }

    #[test]
    fn crate_tokens_are_path_safe() {
        assert_eq!(crate_token(CrateName::Loole), "loole");
        assert_eq!(crate_token(CrateName::Flume), "flume");
        assert_eq!(crate_token(CrateName::AsyncChannel), "async-channel");
        assert_eq!(crate_token(CrateName::Tokio), "tokio");
        assert_eq!(crate_token(CrateName::Kanal), "kanal-std-mutex");
        assert_eq!(
            crate_token(CrateName::CrossbeamChannel),
            "crossbeam-channel"
        );
        for &c in &crate::benchmark::ALL_CRATES {
            let t = crate_token(c);
            assert!(
                t.chars().all(|ch| ch.is_ascii_lowercase() || ch == '-'),
                "path-safe token: {t}"
            );
        }
    }

    #[test]
    fn bench_id_is_combo_cap_crate() {
        assert_eq!(
            bench_id(Combo::SyncSync, Some(10), CrateName::Loole),
            "sync-sync/b10/loole"
        );
        assert_eq!(
            bench_id(Combo::AsyncSync, None, CrateName::Kanal),
            "async-sync/unbounded/kanal-std-mutex"
        );
    }
}
