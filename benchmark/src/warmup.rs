//! Untimed warmup cell: one small transfer per crate before timing starts.
//!
//! Ramps CPU frequency, wakes runtime workers, and warms caches/code paths
//! so the first timed cell does not pay cold-start tax for the second. Runs
//! outside criterion's measurement (called from the bench child before any
//! benchmark group is registered).

use tokio::runtime::Runtime;

use crate::bench_utils::Combo;
use crate::benchmark::{setup_throughput, CrateName};
use crate::message_type::PayloadKind;

/// One untimed SPSC bounded(10) async-async 256-byte transfer of `msg_no`
/// messages. Returns `(sent, received)`; panics via assert on mismatch only
/// in tests — here return the counts and let the caller decide.
pub fn run_once(rt: &Runtime, krate: CrateName, msg_no: usize) -> Option<(usize, usize)> {
    let run = setup_throughput(rt, krate, Combo::AsyncAsync, 1, 1, Some(10), msg_no, 256, PayloadKind::Alloc).ok()?;
    Some(rt.block_on(run.join_all()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn warmup_transfer_moves_all_messages() {
        let rt = runtime();
        for krate in [CrateName::Loole, CrateName::Flume] {
            assert_eq!(run_once(&rt, krate, 1_000), Some((1_000, 1_000)));
        }
    }

    #[test]
    fn warmup_skips_unsupported_cells() {
        let rt = runtime();
        assert_eq!(run_once(&rt, CrateName::CrossbeamChannel, 1_000), None);
    }
}
