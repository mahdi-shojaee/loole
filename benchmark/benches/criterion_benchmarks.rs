//! Criterion benchmark harness for the channel comparison suite.
//!
//! This binary has a thin wrapper `main`: the suite's own flags (see
//! [`benchmark::config::PARAMS`]) are split off, validated, and handed to a
//! re-exec'd copy of this binary as `BENCH_*` env vars. Everything else —
//! criterion's own CLI (`--sample-size`, `--save-baseline`, positional
//! filters, ...) — passes through untouched, because criterion's parser
//! rejects unknown arguments.
//!
//! Benchmark IDs are self-describing so the chart converter
//! (`benchmark/charts/collect-criterion.js`) can rebuild the chart JSON from
//! `target/criterion` alone:
//!
//! ```text
//! <metric>-<size>-n<count>/<topology>-s<S>r<R>/<combo>/<cap>/<crate>
//! ```

use std::path::PathBuf;
use std::process::{exit, Command, ExitStatus, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use benchmark::benchmark::{
    setup_latency, setup_throughput, supports_latency, supports_throughput,
};
use benchmark::chart_update::{should_update_charts, SkipReason};
use benchmark::config::{
    sampling_overrides_from, split_args, Config, SamplingOverrides, BOOL_FLAGS, CHILD_ENV, PARAMS,
};
use benchmark::ids::{
    bench_id, group_id, topology_token, MetricKind, Topology, CAPACITIES, COMBOS,
};

use criterion::{BatchSize, Criterion, Throughput};
use tokio::runtime::Runtime;

/// Criterion's minimum is 10 as well; each iteration already transfers the
/// whole workload, so few samples suffice. Deepen with `--sample-size`,
/// `--measurement-time`, `--warm-up-time` when publishing.
const SAMPLE_SIZE: usize = 10;

/// In-code time budgets (criterion's own defaults are 3s/5s per cell, which
/// makes a full ~570-cell run far too slow). CLI flags override these.
const WARM_UP_TIME: Duration = Duration::from_millis(500);
const MEASUREMENT_TIME: Duration = Duration::from_millis(1500);

fn rt() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| Runtime::new().unwrap())
}

// ---------------------------------------------------------------------------
// Wrapper CLI
// ---------------------------------------------------------------------------

fn main() {
    if std::env::var_os(CHILD_ENV).is_none() {
        wrapper_main();
    }
    // Child process: criterion sees only its own args.
    let mut criterion = Criterion::default().configure_from_args();
    criterion_benchmarks(&mut criterion);
    criterion.final_summary();
}

fn wrapper_main() -> ! {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_custom_help(); // criterion's own help follows from the child.
    }
    match split_args(&args) {
        Ok((env_pairs, passthrough)) => {
            let status = re_exec(&env_pairs, &passthrough);
            maybe_update_charts(&env_pairs, &passthrough, status);
            exit(status.code().unwrap_or(1));
        }
        Err(err) => {
            eprintln!("error: {err}\n");
            print_custom_help();
            exit(2);
        }
    }
}

fn re_exec(env_pairs: &[(&'static str, String)], passthrough: &[String]) -> ExitStatus {
    let exe = std::env::current_exe().expect("current exe");
    let mut cmd = Command::new(exe);
    cmd.args(passthrough).env(CHILD_ENV, "1");
    for (key, value) in env_pairs {
        cmd.env(key, value);
    }
    cmd.status().expect("failed to spawn benchmark process")
}

fn print_custom_help() {
    eprintln!("Benchmark suite options (validated here, then re-exported as env vars):");
    for param in PARAMS {
        let short = param.short.map(|s| format!("{s}, ")).unwrap_or_default();
        eprintln!("    {short}{:<18} [env: {}]", param.long, param.env);
    }
    for flag in BOOL_FLAGS {
        eprintln!("    {:<22} [env: {}]", flag.long, flag.env);
    }
    eprintln!("        metric: throughput | latency | both (default: both)");
    eprintln!(
        "        crates: space/comma-separated; default: all \
         (loole, flume, async-channel, tokio, kanal, crossbeam-channel)"
    );
    eprintln!();
}

// ---------------------------------------------------------------------------
// Automatic chart update
// ---------------------------------------------------------------------------

/// Full-matrix runs update the charts automatically. Anything narrower
/// (metric/crate subsetting, a criterion bench filter) is left alone so
/// committed charts are never clobbered by partial data; the skip message
/// points at the manual command.
fn maybe_update_charts(
    env_pairs: &[(&'static str, String)],
    passthrough: &[String],
    status: ExitStatus,
) {
    let cfg = match Config::from_lookup(|name| {
        env_pairs
            .iter()
            .rev()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.clone())
            .or_else(|| std::env::var(name).ok())
    }) {
        Ok(cfg) => cfg,
        Err(_) => return, // wrapper already validated every value
    };
    match should_update_charts(&cfg, passthrough, status.success()) {
        Ok(()) => run_chart_update(cfg.message_size),
        Err(SkipReason::MetaCommand) => {}
        Err(reason) => eprintln!(
            "charts not updated ({reason}); full runs update them automatically, \
             or run manually:\n  cd benchmark/charts && npm run update -- {}",
            cfg.message_size
        ),
    }
}

fn charts_dir() -> Option<PathBuf> {
    // cargo runs bench binaries with cwd = package root (benchmark/), while
    // `cargo bench` from the shell keeps the caller's cwd; probe both, plus
    // a bare charts/ cwd.
    let cwd = std::env::current_dir().ok()?;
    [
        cwd.join("benchmark").join("charts"),
        cwd.join("charts"),
        cwd,
    ]
    .into_iter()
    .find(|dir| dir.join("package.json").is_file())
}

fn command_ok(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn run_chart_update(message_size: usize) {
    let Some(charts_dir) = charts_dir() else {
        eprintln!("charts not updated: benchmark/charts not found");
        return;
    };
    if !command_ok("node", &["--version"]) || !command_ok("npm", &["--version"]) {
        eprintln!("charts not updated: Node.js is required (https://nodejs.org)");
        return;
    }
    println!("updating charts (message size {message_size})…");
    if !charts_dir.join("node_modules").is_dir() {
        let status = Command::new("npm")
            .arg("install")
            .current_dir(&charts_dir)
            .status();
        if !matches!(status, Ok(s) if s.success()) {
            eprintln!("charts not updated: npm install failed");
            return;
        }
    }
    let status = Command::new("npm")
        .args(["run", "update", "--", &message_size.to_string()])
        .current_dir(&charts_dir)
        .status();
    if !matches!(status, Ok(s) if s.success()) {
        eprintln!(
            "charts update failed; retry manually:\n  cd benchmark/charts && npm run update -- {message_size}"
        );
    }
}

// ---------------------------------------------------------------------------
// Benchmark registration
// ---------------------------------------------------------------------------

fn criterion_benchmarks(c: &mut Criterion) {
    let cfg = Config::from_env().unwrap_or_else(|err| {
        eprintln!("error: {err}");
        exit(2);
    });
    // In-code sampling defaults apply only where the CLI didn't pin a value
    // (group-level settings would otherwise override criterion's own CLI).
    let args: Vec<String> = std::env::args().collect();
    let overrides = sampling_overrides_from(&args);
    // Mark this run's start; collect-criterion.js only trusts estimates
    // written after it, so stale cells can never leak into charts. Meta
    // operations (--list/--help/…) write no estimates, so they leave the
    // markers of the last real run alone.
    if !benchmark::chart_update::is_meta_run(&args) {
        if let Err(err) = benchmark::run_marker::record_run_start(&cfg) {
            eprintln!("error: cannot record run marker: {err}");
            exit(2);
        }
    }
    if cfg.metric.includes_throughput() {
        throughput_groups(c, &cfg, overrides);
    }
    if cfg.metric.includes_latency() {
        latency_group(c, &cfg, overrides);
    }
}

/// Sets the quick in-code sampling defaults, except for knobs pinned via CLI.
fn apply_group_defaults<M: criterion::measurement::Measurement>(
    group: &mut criterion::BenchmarkGroup<'_, M>,
    overrides: SamplingOverrides,
) {
    if !overrides.sample_size {
        group.sample_size(SAMPLE_SIZE);
    }
    if !overrides.warm_up_time {
        group.warm_up_time(WARM_UP_TIME);
    }
    if !overrides.measurement_time {
        group.measurement_time(MEASUREMENT_TIME);
    }
}

fn topologies(cfg: &Config) -> [(Topology, usize, usize); 3] {
    [
        (Topology::Spsc, 1, 1),
        (Topology::Mpsc, cfg.senders, 1),
        (Topology::Mpmc, cfg.senders, cfg.receivers),
    ]
}

fn throughput_groups(c: &mut Criterion, cfg: &Config, overrides: SamplingOverrides) {
    let gid = group_id(MetricKind::Throughput, cfg.message_size, cfg.messages_no);
    for (topology, senders, receivers) in topologies(cfg) {
        let mut group = c.benchmark_group(format!(
            "{gid}/{}",
            topology_token(topology, senders, receivers)
        ));
        apply_group_defaults(&mut group, overrides);
        group.throughput(Throughput::Elements(cfg.messages_no as u64));
        for (combo, _) in COMBOS {
            for (cap, _) in CAPACITIES {
                for &krate in &cfg.crates {
                    if !supports_throughput(krate, combo, receivers, cap) {
                        continue;
                    }
                    group.bench_function(bench_id(combo, cap, krate), |b| {
                        b.iter_batched(
                            || {
                                setup_throughput(
                                    rt(),
                                    krate,
                                    combo,
                                    senders,
                                    receivers,
                                    cap,
                                    cfg.messages_no,
                                    cfg.message_size,
                                )
                                .expect("cell checked by supports_throughput")
                            },
                            // The transfer happens (and completes) on join:
                            // the exact `t0`-after-spawn semantics of the old runner.
                            |run| rt().block_on(run.join_all()),
                            BatchSize::PerIteration,
                        );
                    });
                }
            }
        }
        group.finish();
    }
}

fn latency_group(c: &mut Criterion, cfg: &Config, overrides: SamplingOverrides) {
    let gid = group_id(MetricKind::Latency, cfg.message_size, cfg.latency_rounds);
    let mut group = c.benchmark_group(format!("{gid}/{}", topology_token(Topology::Spsc, 1, 1)));
    apply_group_defaults(&mut group, overrides);
    group.throughput(Throughput::Elements(cfg.latency_rounds as u64));
    for (combo, _) in COMBOS {
        for (cap, _) in CAPACITIES {
            for &krate in &cfg.crates {
                if !supports_latency(krate, combo, cap) {
                    continue;
                }
                group.bench_function(bench_id(combo, cap, krate), |b| {
                    b.iter_batched(
                        || {
                            setup_latency(
                                rt(),
                                krate,
                                combo,
                                cap,
                                cfg.message_size,
                                cfg.latency_warmup,
                            )
                            .expect("cell checked by supports_latency")
                        },
                        |mut probe| probe.run(rt(), cfg.latency_rounds),
                        BatchSize::PerIteration,
                    );
                });
            }
        }
    }
    group.finish();
}
