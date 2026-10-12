//! Per-message-size run markers: `<criterion-dir>/runs/last-<size>-<payload>.json`.
//!
//! The benchmark binary overwrites the marker for its message size at the
//! start of every run (constant size, one tiny file per size ever measured).
//! The chart converter only trusts criterion estimates written *after* the
//! marker's timestamp, so charts always reflect the latest run of that size
//! and `target/criterion` never needs manual cleaning.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;

/// Subdirectory of the criterion output dir holding the per-size markers.
pub const RUNS_DIR: &str = "runs";

/// Resolves criterion's output dir: `$CRITERION_HOME` if set, else derived
/// from the current executable's location (…/target/release/deps/<bench>).
pub fn criterion_dir() -> io::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("CRITERION_HOME") {
        return Ok(PathBuf::from(dir));
    }
    criterion_dir_from_exe(&std::env::current_exe()?)
}

fn criterion_dir_from_exe(exe: &Path) -> io::Result<PathBuf> {
    // exe = <target>/<profile>/deps/<bench>-<hash>
    exe.ancestors()
        .nth(3)
        .map(|target| target.join("criterion"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "cannot locate target dir"))
}

/// Records this run's start; overwrites the marker for `cfg.message_size`.
pub fn record_run_start(cfg: &Config) -> io::Result<PathBuf> {
    record_run_start_at(&criterion_dir()?, cfg)
}

pub fn record_run_start_at(criterion_dir: &Path, cfg: &Config) -> io::Result<PathBuf> {
    let started_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let runs_dir = criterion_dir.join(RUNS_DIR);
    fs::create_dir_all(&runs_dir)?;
    let path = runs_dir.join(format!(
        "last-{}-{}.json",
        cfg.message_size,
        cfg.payload.token()
    ));
    let crates = cfg
        .crates
        .iter()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(",");
    // Machine snapshot for fairness auditing: a suspicious ranking can be
    // checked against the conditions each run measured under. Uses only
    // portable std APIs (no OS-specific sensors) so markers stay identical
    // across Linux/macOS/Windows.
    let parallelism = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(0);
    let payload = format!(
        concat!(
            "{{\"started_at\":{started_ms},\"message_size\":{message_size},",
            "\"messages_no\":{messages_no},\"senders\":{senders},\"receivers\":{receivers},",
            "\"metric\":\"{metric}\",\"latency_rounds\":{latency_rounds},\"crates\":\"{crates}\",",
            "\"parallelism\":{parallelism}}}\n"
        ),
        started_ms = started_ms,
        message_size = cfg.message_size,
        messages_no = cfg.messages_no,
        senders = cfg.senders,
        receivers = cfg.receivers,
        metric = cfg.metric,
        latency_rounds = cfg.latency_rounds,
        crates = crates,
        parallelism = parallelism,
    );
    // Temp-then-rename so an interrupted write can never leave a truncated
    // marker (which would lock the converter out of the run's data with a
    // hard parse error) — readers see the old or the new marker, never a
    // half-written one.
    let tmp = path.with_extension("json.tmp");
    if let Err(err) = fs::write(&tmp, payload) {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }
    fs::rename(&tmp, &path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark::CrateName;
    use crate::config::Metric;
    use std::fs;

    fn test_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("loole-bench-marker-{}-{}", std::process::id(), tag));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writes_marker_for_message_size() {
        let dir = test_dir("write");
        let mut cfg = Config::default();
        cfg.message_size = 32;
        cfg.payload = crate::message_type::PayloadKind::Touched;
        let path = record_run_start_at(&dir, &cfg).unwrap();
        assert_eq!(path, dir.join("runs").join("last-32-touched.json"));
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("\"message_size\":32"), "{body}");
        assert!(body.contains("\"started_at\":"), "{body}");
        assert!(body.contains("\"metric\":\"both\""), "{body}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn overwrites_and_timestamps_advance() {
        let dir = test_dir("advance");
        let cfg = Config::default();
        record_run_start_at(&dir, &cfg).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        record_run_start_at(&dir, &cfg).unwrap();
        let body = fs::read_to_string(dir.join("runs").join("last-256-touched.json")).unwrap();
        // Constant size: exactly one marker, and it parses as one line of JSON.
        assert_eq!(body.lines().count(), 1);
        assert!(body.starts_with('{'));
        // Atomic write: no temp file left behind.
        let runs: Vec<_> = fs::read_dir(dir.join("runs")).unwrap().collect();
        assert_eq!(runs.len(), 1, "only the marker file may remain: {runs:?}");
    }

    #[test]
    fn marker_echoes_effective_config() {
        let dir = test_dir("config");
        let cfg = Config {
            message_size: 2048,
            messages_no: 5_000,
            senders: 4,
            receivers: 2,
            metric: Metric::Latency,
            crates: vec![CrateName::Loole, CrateName::Tokio],
            latency_rounds: 7_000,
            latency_warmup: 100,
            no_charts: false,
            cooldown_secs: 0,
            worker_threads: 0,
            isolate_crates: false,
            no_warmup_cell: false,
            payload: crate::message_type::PayloadKind::Touched,
        };
        let path = record_run_start_at(&dir, &cfg).unwrap();
        let body = fs::read_to_string(path).unwrap();
        for needle in [
            r#""message_size":2048"#,
            r#""messages_no":5000"#,
            r#""senders":4"#,
            r#""receivers":2"#,
            r#""metric":"latency""#,
            r#""latency_rounds":7000"#,
            r#""crates":"loole,tokio""#,
        ] {
            assert!(body.contains(needle), "missing {needle} in {body}");
        }
        // Machine snapshot: echoes this host's parallelism, whatever it is.
        let expected = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0);
        assert!(
            body.contains(&format!("\"parallelism\":{expected}")),
            "missing parallelism snapshot in {body}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn criterion_dir_derives_from_exe_path() {
        let exe = Path::new("/repo/target/release/deps/criterion_benchmarks-abc123");
        assert_eq!(
            criterion_dir_from_exe(exe).unwrap(),
            PathBuf::from("/repo/target/criterion")
        );
    }
}
