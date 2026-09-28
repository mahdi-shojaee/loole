//! Machine/run descriptor for fairness auditing.
//!
//! Every benchmark invocation writes one `machine-<size>-<payload>-<unix_ms>.json`
//! via `write_machine_json_at` next to its run marker so a suspicious ranking
//! can be checked against the conditions it measured under: governor, per-CPU frequencies, topology, affinity, load,
//! memory, git revision, and the exact invocation. Readers use only portable
//! std APIs plus best-effort `/proc` and `/sys` reads; every field falls back
//! to `"unknown"` (never an error), so markers stay comparable across
//! Linux/macOS/Windows.

use std::io;
use std::path::{Path, PathBuf};

/// Escape a string for embedding in JSON double quotes.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn read_first_line(path: &str) -> String {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|b| b.lines().next().map(|l| l.trim().to_string()))
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn read_all_cpu_freqs() -> Vec<String> {
    let mut freqs = Vec::new();
    for cpu in 0..4096 {
        let p = format!("/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_cur_freq");
        match std::fs::read_to_string(&p) {
            Ok(b) => freqs.push(b.trim().to_string()),
            Err(_) => break,
        }
    }
    if freqs.is_empty() {
        vec!["unknown".to_string()]
    } else {
        freqs
    }
}

fn cpu_model() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|b| {
            b.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split_once(':'))
                .map(|(_, v)| v.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn git_revision() -> (String, bool) {
    let sha = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok()
            } else {
                None
            }
        })
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let dirty = std::process::Command::new("git")
        .args(["status", "--short"])
        .output()
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false);
    (sha, dirty)
}

/// All fields are plain strings / string lists so the writer needs no serde.
pub struct MachineDesc {
    pub timestamp_ms: String,
    pub payload: String,
    pub invocation: String,
    pub cpu_model: String,
    pub cpu_count: String,
    pub governor: String,
    pub cur_freqs_khz: Vec<String>,
    pub affinity: String,
    pub loadavg: String,
    pub mem_total_kb: String,
    pub git_sha: String,
    pub git_dirty: String,
    pub extra_env: Vec<(String, String)>,
}

/// `extra_env` receives the suite's `BENCH_*` assignments so the descriptor
/// records the exact workload alongside the machine state.
pub fn collect(invocation: &str, payload: &str, extra_env: &[(&str, String)]) -> MachineDesc {
    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let governors: Vec<String> = {
        let mut set = std::collections::BTreeSet::new();
        for cpu in 0..4096 {
            let p = format!("/sys/devices/system/cpu/cpu{cpu}/cpufreq/scaling_governor");
            match std::fs::read_to_string(&p) {
                Ok(b) => {
                    set.insert(b.trim().to_string());
                }
                Err(_) => break,
            }
        }
        if set.is_empty() {
            vec!["unknown".to_string()]
        } else {
            set.into_iter().collect()
        }
    };
    let affinity = read_first_line("/proc/self/status")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let affinity = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|b| {
            b.lines()
                .find(|l| l.starts_with("Cpus_allowed_list:"))
                .and_then(|l| l.split_once(':'))
                .map(|(_, v)| v.trim().to_string())
        })
        .unwrap_or(affinity);
    let loadavg = read_first_line("/proc/loadavg");
    let mem_total_kb = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|b| {
            b.lines()
                .find(|l| l.starts_with("MemTotal:"))
                .map(|l| l.to_string())
        })
        .unwrap_or_else(|| "unknown".to_string());
    let cpu_count = std::thread::available_parallelism()
        .map(|n| n.get().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let (git_sha, dirty) = git_revision();
    MachineDesc {
        timestamp_ms,
        payload: payload.to_string(),
        invocation: invocation.to_string(),
        cpu_model: cpu_model(),
        cpu_count,
        governor: governors.join(","),
        cur_freqs_khz: read_all_cpu_freqs(),
        affinity,
        loadavg,
        mem_total_kb,
        git_sha,
        git_dirty: dirty.to_string(),
        extra_env: extra_env
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    }
}

/// Collision-free sidecar filename for one wrapper invocation:
/// `machine-<message_size>-<payload>-<unix_ms>.json`, so repeated runs over
/// sizes/payloads never overwrite each other's audit trail.
pub fn machine_filename(message_size: usize, payload: &str, unix_ms: &str) -> String {
    format!("machine-{message_size}-{payload}-{unix_ms}.json")
}

/// Writes the descriptor into `dir` (created if missing) under `filename`
/// and returns its path.
pub fn write_machine_json_at(
    dir: &Path,
    filename: &str,
    desc: &MachineDesc,
) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let freqs = desc
        .cur_freqs_khz
        .iter()
        .map(|f| format!("\"{}\"", esc(f)))
        .collect::<Vec<_>>()
        .join(", ");
    let env = desc
        .extra_env
        .iter()
        .map(|(k, v)| format!("\"{}\": \"{}\"", esc(k), esc(v)))
        .collect::<Vec<_>>()
        .join(", ");
    let body = format!(
        "{{\n  \"timestamp_ms\": \"{}\",\n  \"payload\": \"{}\",\n  \"invocation\": \"{}\",\n  \"cpu_model\": \"{}\",\n  \"cpu_count\": \"{}\",\n  \"governor\": \"{}\",\n  \"cur_freqs_khz\": [{}],\n  \"affinity\": \"{}\",\n  \"loadavg\": \"{}\",\n  \"mem_total_kb\": \"{}\",\n  \"git_sha\": \"{}\",\n  \"git_dirty\": \"{}\",\n  \"extra_env\": {{{}}}\n}}\n",
        esc(&desc.timestamp_ms),
        esc(&desc.payload),
        esc(&desc.invocation),
        esc(&desc.cpu_model),
        esc(&desc.cpu_count),
        esc(&desc.governor),
        freqs,
        esc(&desc.affinity),
        esc(&desc.loadavg),
        esc(&desc.mem_total_kb),
        esc(&desc.git_sha),
        esc(&desc.git_dirty),
        env
    );
    let path = dir.join(filename);
    std::fs::write(&path, body)?;
    Ok(path)
}

/// Writes `machine.json` into `dir` (created if missing) and returns its path.
pub fn write_machine_json(dir: &Path, desc: &MachineDesc) -> io::Result<PathBuf> {
    write_machine_json_at(dir, "machine.json", desc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esc_handles_quotes_backslashes_and_control_chars() {
        assert_eq!(esc("a\"b\\c"), "a\\\"b\\\\c");
        assert_eq!(esc("x\ny\tz"), "x\\ny\\tz");
        assert_eq!(esc("plain-abc_123"), "plain-abc_123");
    }

    #[test]
    fn collect_fills_every_field() {
        let d = collect("bench --test", "alloc", &[]);
        assert!(!d.timestamp_ms.is_empty());
        assert!(!d.cpu_model.is_empty());
        assert!(!d.governor.is_empty());
        assert!(!d.affinity.is_empty());
        assert!(!d.invocation.is_empty());
        assert_eq!(d.payload, "alloc");
    }

    #[test]
    fn write_roundtrip_in_tempdir() {
        let dir = std::env::temp_dir().join(format!("loole-machine-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = collect("bench --test", "shared", &[("BENCH_FOO", "bar".to_string())]);
        let path = write_machine_json(&dir, &d).unwrap();
        assert_eq!(path, dir.join("machine.json"));
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("\"payload\": \"shared\""));
        assert!(body.contains("\"BENCH_FOO\": \"bar\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn machine_filename_is_collision_free_and_write_at_uses_it() {
        assert_eq!(
            machine_filename(256, "alloc", "123"),
            "machine-256-alloc-123.json"
        );
        let dir = std::env::temp_dir().join(format!(
            "loole-machine-at-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = collect("bench --test", "alloc", &[]);
        let name = machine_filename(256, "alloc", &d.timestamp_ms);
        let path = write_machine_json_at(&dir, &name, &d).unwrap();
        assert_eq!(path, dir.join(&name));
        assert!(path.is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
