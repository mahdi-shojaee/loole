//! Per-run latency-distribution sidecar: percentile tables built from the
//! per-cell histograms recorded during throughput runs.
//!
//! Criterion's own output cannot express custom distributions, so each
//! measuring process writes one
//! `latency-dist-<size>-<payload>-<unix_ms>-<pid>.json` next to the run
//! markers (see [`crate::run_marker`]). Filenames are collision-free across
//! isolate-mode children (pid) and repeated runs (timestamp), so every child
//! writes its own file and the chart converter merges the newest sidecars
//! per (size, payload) the same newest-wins way it merges estimates.
//!
//! Quantile keys use underscores (`p99_9`) so every key is jq-friendly
//! without quoting.

use std::io;
use std::path::{Path, PathBuf};

use hdrhistogram::Histogram;

use crate::machine::esc;

/// Charted quantiles, ascending: p10/p50/p95/p99/p99.9.
pub const DIST_QUANTILES: [f64; 5] = [10.0, 50.0, 95.0, 99.0, 99.9];

/// One throughput cell's merged latency distribution, in nanos.
pub struct CellDist {
    pub topology: String,
    pub senders: usize,
    pub receivers: usize,
    pub combo: String,
    pub cap: String,
    pub krate: String,
    pub messages_no: usize,
    pub sample_every: usize,
    pub hist: Histogram<u64>,
}

/// Percentile table for one cell, in nanos.
pub struct CellQuantiles {
    pub count: u64,
    pub min: u64,
    pub mean: f64,
    pub max: u64,
    /// In [`DIST_QUANTILES`] order.
    pub p: [u64; 5],
}

pub fn quantiles(hist: &Histogram<u64>) -> CellQuantiles {
    CellQuantiles {
        count: hist.len(),
        min: hist.min(),
        mean: hist.mean(),
        max: hist.max(),
        p: DIST_QUANTILES.map(|q| hist.value_at_percentile(q)),
    }
}

/// Collision-free sidecar filename for one measuring process:
/// `latency-dist-<size>-<payload>-<unix_ms>-<pid>.json`.
pub fn dist_filename(message_size: usize, payload: &str, unix_ms: &str) -> String {
    format!(
        "latency-dist-{message_size}-{payload}-{unix_ms}-{}.json",
        std::process::id()
    )
}

/// Writes the sidecar into `dir` (created if missing) under `filename` and
/// returns its path. Cells with no recorded samples are omitted: a run that
/// measured nothing (meta invocations, shared mode) leaves no chart input
/// behind.
pub fn write_dist_sidecar_at(
    dir: &Path,
    filename: &str,
    timestamp_ms: &str,
    message_size: usize,
    payload: &str,
    cells: &[CellDist],
) -> io::Result<PathBuf> {
    let cells_json: Vec<String> = cells
        .iter()
        .filter(|c| c.hist.len() > 0)
        .map(|c| {
            let q = quantiles(&c.hist);
            format!(
                "{{\"topology\":\"{}\",\"senders\":{},\"receivers\":{},\"combo\":\"{}\",\"cap\":\"{}\",\"crate\":\"{}\",\"messages_no\":{},\"sample_every\":{},\"count\":{},\"min\":{},\"mean\":{:.2},\"max\":{},\"p10\":{},\"p50\":{},\"p95\":{},\"p99\":{},\"p99_9\":{}}}",
                esc(&c.topology),
                c.senders,
                c.receivers,
                esc(&c.combo),
                esc(&c.cap),
                esc(&c.krate),
                c.messages_no,
                c.sample_every,
                q.count,
                q.min,
                q.mean,
                q.max,
                q.p[0],
                q.p[1],
                q.p[2],
                q.p[3],
                q.p[4],
            )
        })
        .collect();
    let body = format!(
        "{{\n  \"timestamp_ms\": \"{}\",\n  \"message_size\": {},\n  \"payload\": \"{}\",\n  \"quantiles\": [10.0, 50.0, 95.0, 99.0, 99.9],\n  \"cells\": [\n{}\n  ]\n}}\n",
        esc(timestamp_ms),
        message_size,
        esc(payload),
        cells_json.join(",\n"),
    );
    std::fs::create_dir_all(dir)?;
    let path = dir.join(filename);
    // Temp-then-rename so an interrupted write can never leave a truncated
    // sidecar in the audit trail.
    let tmp = path.with_extension("json.tmp");
    if let Err(err) = std::fs::write(&tmp, body) {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform_hist() -> Histogram<u64> {
        let mut hist = Histogram::<u64>::new(3).expect("valid sigfig");
        for v in 1..=1000u64 {
            hist.record(v).expect("in range");
        }
        hist
    }

    #[test]
    fn quantiles_on_synthetic_uniform_histogram() {
        let q = quantiles(&uniform_hist());
        assert_eq!((q.count, q.min, q.max), (1000, 1, 1000));
        assert!((495..=505).contains(&q.p[1]), "p50 ≈ 500, got {}", q.p[1]);
        assert!(q.p[0] <= q.p[1] && q.p[1] <= q.p[2] && q.p[2] <= q.p[3] && q.p[3] <= q.p[4]);
        assert!((495.0..=505.0).contains(&q.mean));
    }

    #[test]
    fn dist_filename_is_collision_free() {
        assert_eq!(
            dist_filename(256, "touched", "123"),
            format!("latency-dist-256-touched-123-{}.json", std::process::id())
        );
    }

    #[test]
    fn write_roundtrip_omits_empty_cells() {
        let dir = std::env::temp_dir().join(format!("loole-dist-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let empty = CellDist {
            topology: "spsc".to_string(),
            senders: 1,
            receivers: 1,
            combo: "sync-sync".to_string(),
            cap: "b10".to_string(),
            krate: "loole".to_string(),
            messages_no: 1000,
            sample_every: 1,
            hist: Histogram::<u64>::new(3).expect("valid sigfig"),
        };
        let full = CellDist {
            topology: "spsc".to_string(),
            senders: 1,
            receivers: 1,
            combo: "async-async".to_string(),
            cap: "unbounded".to_string(),
            krate: "flume".to_string(),
            messages_no: 1000,
            sample_every: 1,
            hist: uniform_hist(),
        };
        let path = write_dist_sidecar_at(&dir, "d.json", "123", 256, "touched", &[empty, full])
            .unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("\"payload\": \"touched\""));
        assert!(body.contains("\"crate\":\"flume\""));
        assert!(!body.contains("\"crate\":\"loole\""), "empty cell omitted");
        assert!(body.contains("\"p99_9\""));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
