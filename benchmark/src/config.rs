//! Configuration for the criterion bench harness.
//!
//! Sources, in increasing precedence: built-in defaults < `BENCH_*` env vars <
//! CLI flags (the bench binary's wrapper converts CLI flags into `BENCH_*` env
//! assignments for its re-exec'd child; criterion never sees them).

use std::str::FromStr;

use crate::benchmark::{CrateName, ALL_CRATES};
use crate::message_type::PayloadKind;

pub const DEFAULT_MESSAGE_SIZE: usize = 256;
pub const DEFAULT_MESSAGES_NO: usize = 100_000;
pub const DEFAULT_SENDERS: usize = 1_000;
pub const DEFAULT_RECEIVERS: usize = 10;
pub const DEFAULT_LATENCY_ROUNDS: usize = 100_000;
pub const DEFAULT_LATENCY_WARMUP: usize = 1_000;

/// Set in the environment of the re-exec'd bench process so it skips the
/// wrapper CLI and hands its args to criterion directly.
pub const CHILD_ENV: &str = "LOOLE_BENCH_CHILD";

/// Which metric suite(s) to register benchmarks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Metric {
    Throughput,
    Latency,
    #[default]
    Both,
}

impl Metric {
    pub fn includes_throughput(self) -> bool {
        matches!(self, Metric::Throughput | Metric::Both)
    }

    pub fn includes_latency(self) -> bool {
        matches!(self, Metric::Latency | Metric::Both)
    }
}

impl FromStr for Metric {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "throughput" => Ok(Metric::Throughput),
            "latency" => Ok(Metric::Latency),
            "both" => Ok(Metric::Both),
            other => Err(format!(
                "unknown metric '{other}' (valid: throughput, latency, both)"
            )),
        }
    }
}

impl std::fmt::Display for Metric {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Metric::Throughput => write!(f, "throughput"),
            Metric::Latency => write!(f, "latency"),
            Metric::Both => write!(f, "both"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub message_size: usize,
    pub messages_no: usize,
    pub senders: usize,
    pub receivers: usize,
    pub metric: Metric,
    pub crates: Vec<CrateName>,
    pub latency_rounds: usize,
    pub latency_warmup: usize,
    /// Skip the automatic chart update after a full run.
    pub no_charts: bool,
    /// Idle seconds between benchmark groups (topologies) and before the
    /// latency group so the machine cools down. High-churn cells heat the
    /// CPU; without a cooldown, later groups measure throttled performance
    /// and comparisons across runs stop being fair. Zero disables.
    pub cooldown_secs: u64,
    /// Tokio worker threads for the shared runtime. Zero (default) keeps
    /// `Runtime::new()` behavior (one worker per CPU).
    pub worker_threads: usize,
    /// Run each selected crate in its own process (fairness isolation).
    pub isolate_crates: bool,
    /// Skip the pre-timing warmup cell.
    pub no_warmup_cell: bool,
    /// Which payload construction the senders use.
    pub payload: PayloadKind,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            message_size: DEFAULT_MESSAGE_SIZE,
            messages_no: DEFAULT_MESSAGES_NO,
            senders: DEFAULT_SENDERS,
            receivers: DEFAULT_RECEIVERS,
            metric: Metric::Both,
            crates: ALL_CRATES.to_vec(),
            latency_rounds: DEFAULT_LATENCY_ROUNDS,
            latency_warmup: DEFAULT_LATENCY_WARMUP,
            no_charts: false,
            cooldown_secs: 0,
            worker_threads: 0,
            isolate_crates: false,
            no_warmup_cell: false,
            payload: PayloadKind::Alloc,
        }
    }
}

/// One configurable parameter and the knobs that set it.
pub struct Param {
    pub long: &'static str,
    pub short: Option<&'static str>,
    pub env: &'static str,
}

pub const PARAMS: &[Param] = &[
    Param {
        long: "--message-size",
        short: Some("-m"),
        env: "BENCH_MESSAGE_SIZE",
    },
    Param {
        long: "--messages-no",
        short: Some("-n"),
        env: "BENCH_MSG_NO",
    },
    Param {
        long: "--max-senders",
        short: None,
        env: "BENCH_SENDERS",
    },
    Param {
        long: "--max-receivers",
        short: None,
        env: "BENCH_RECEIVERS",
    },
    Param {
        long: "--metric",
        short: None,
        env: "BENCH_METRIC",
    },
    Param {
        long: "--crates",
        short: None,
        env: "BENCH_CRATES",
    },
    Param {
        long: "--latency-rounds",
        short: None,
        env: "BENCH_LATENCY_ROUNDS",
    },
    Param {
        long: "--latency-warmup",
        short: None,
        env: "BENCH_LATENCY_WARMUP",
    },
    Param {
        long: "--cooldown-secs",
        short: None,
        env: "BENCH_COOLDOWN_SECS",
    },
    Param {
        long: "--worker-threads",
        short: None,
        env: "BENCH_WORKER_THREADS",
    },
    Param {
        long: "--payload",
        short: None,
        env: "BENCH_PAYLOAD",
    },
];

/// Value-less boolean suite flags (same shape as [`Param`]; presence = true).
pub const BOOL_FLAGS: &[Param] = &[
    Param {
        long: "--no-charts",
        short: None,
        env: "BENCH_NO_CHARTS",
    },
    Param {
        long: "--isolate-crates",
        short: None,
        env: "BENCH_ISOLATE_CRATES",
    },
    Param {
        long: "--no-warmup-cell",
        short: None,
        env: "BENCH_NO_WARMUP_CELL",
    },
];

/// Which criterion sampling knobs the user set on the CLI. The harness sets
/// its own quick in-code defaults per benchmark group, so it must skip any
/// knob the user pinned — group-level settings would otherwise win over
/// criterion's CLI parsing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SamplingOverrides {
    pub sample_size: bool,
    pub warm_up_time: bool,
    pub measurement_time: bool,
}

impl SamplingOverrides {
    pub fn any(&self) -> bool {
        self.sample_size || self.warm_up_time || self.measurement_time
    }
}

/// Detects `--sample-size` / `--warm-up-time` / `--measurement-time` in args,
/// in both `--flag value` and `--flag=value` forms.
pub fn sampling_overrides_from(args: &[String]) -> SamplingOverrides {
    let mut out = SamplingOverrides::default();
    for arg in args {
        for (flag, field) in [
            ("--sample-size", &mut out.sample_size),
            ("--warm-up-time", &mut out.warm_up_time),
            ("--measurement-time", &mut out.measurement_time),
        ] {
            if arg == flag || arg.starts_with(&format!("{flag}=")) {
                *field = true;
            }
        }
    }
    out
}

impl Config {
    pub fn from_env() -> Result<Config, String> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Config, String> {
        let mut cfg = Config::default();
        for param in PARAMS.iter().chain(BOOL_FLAGS) {
            if let Some(value) = get(param.env) {
                apply_value(&mut cfg, param.env, &value)?;
            }
        }
        Ok(cfg)
    }
}

fn parse_count(env: &str, value: &str) -> Result<usize, String> {
    let n: usize = value
        .parse()
        .map_err(|_| format!("{env}: '{value}' is not a number"))?;
    if n < 1 {
        return Err(format!("{env}: must be >= 1"));
    }
    Ok(n)
}

fn parse_bool(env: &str, value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        other => Err(format!("{env}: '{other}' is not a boolean")),
    }
}

/// Parses a non-negative integer. Unlike [`parse_count`], zero is valid:
/// a zero cooldown disables the inter-group sleep.
fn parse_nonnegative(env: &str, value: &str) -> Result<u64, String> {
    value
        .trim()
        .parse()
        .map_err(|_| format!("{env}: '{value}' is not a non-negative integer"))
}

/// Canonicalizes one parameter value, assigning it to `cfg`.
fn apply_value(cfg: &mut Config, env: &'static str, value: &str) -> Result<(), String> {
    match env {
        "BENCH_MESSAGE_SIZE" => cfg.message_size = parse_count(env, value)?,
        "BENCH_MSG_NO" => cfg.messages_no = parse_count(env, value)?,
        "BENCH_SENDERS" => cfg.senders = parse_count(env, value)?,
        "BENCH_RECEIVERS" => cfg.receivers = parse_count(env, value)?,
        "BENCH_METRIC" => cfg.metric = Metric::from_str(value)?,
        "BENCH_CRATES" => cfg.crates = parse_crates(value)?,
        "BENCH_LATENCY_ROUNDS" => cfg.latency_rounds = parse_count(env, value)?,
        "BENCH_LATENCY_WARMUP" => cfg.latency_warmup = parse_count(env, value)?,
        "BENCH_NO_CHARTS" => cfg.no_charts = parse_bool(env, value)?,
        "BENCH_COOLDOWN_SECS" => cfg.cooldown_secs = parse_nonnegative(env, value)?,
        "BENCH_WORKER_THREADS" => {
            cfg.worker_threads = parse_nonnegative(env, value)? as usize
        }
        "BENCH_ISOLATE_CRATES" => cfg.isolate_crates = parse_bool(env, value)?,
        "BENCH_NO_WARMUP_CELL" => cfg.no_warmup_cell = parse_bool(env, value)?,
        "BENCH_PAYLOAD" => cfg.payload = PayloadKind::from_env(value)?,
        other => return Err(format!("unknown parameter environment name: {other}")),
    }
    Ok(())
}

/// Parses a space/comma-separated crate list into a deduplicated slice in
/// canonical order. The special value "all" (or an empty list) selects every
/// crate. Unknown names are hard errors.
///
/// Note: the full name "kanal (std-mutex)" contains a space, so it is only
/// recognized as a whole comma-separated part; prefer the alias "kanal".
pub fn parse_crates(s: &str) -> Result<Vec<CrateName>, String> {
    let s = s.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("all") {
        return Ok(ALL_CRATES.to_vec());
    }
    let mut selected = Vec::new();
    let mut add = |name: &str| -> Result<(), String> {
        let crate_name = CrateName::from_str(name)?;
        if !selected.contains(&crate_name) {
            selected.push(crate_name);
        }
        Ok(())
    };
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        // Try the whole part first so "kanal (std-mutex)" survives comma input.
        if CrateName::from_str(part).is_ok() {
            add(part)?;
        } else {
            for name in part.split_whitespace() {
                add(name)?;
            }
        }
    }
    if selected.is_empty() {
        return Ok(ALL_CRATES.to_vec());
    }
    // Reorder into canonical order so the JSON header is deterministic.
    Ok(ALL_CRATES
        .iter()
        .copied()
        .filter(|c| selected.contains(c))
        .collect())
}

/// Splits process args into (env assignments for the re-exec'd child, args to
/// hand to criterion). Custom flags (see [`PARAMS`]) are consumed and
/// validated; everything else passes through in order, so criterion's own CLI
/// keeps working end-to-end. Accepted flag forms: `--flag value`,
/// `--flag=value`, `-s value`, `-svalue`.
pub fn split_args(args: &[String]) -> Result<(Vec<(&'static str, String)>, Vec<String>), String> {
    let mut env: Vec<(&'static str, String)> = Vec::new();
    let mut rest: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match match_custom(args, i)? {
            Some((env_pair, consumed)) => {
                env.push(env_pair);
                i += consumed;
            }
            None => {
                rest.push(arg.clone());
                i += 1;
            }
        }
    }
    Ok((env, rest))
}

/// If `args[start]` begins a custom flag, validates its value and returns the
/// canonical env assignment plus how many args were consumed.
fn match_custom(
    args: &[String],
    start: usize,
) -> Result<Option<((&'static str, String), usize)>, String> {
    let arg = &args[start];
    for flag in BOOL_FLAGS {
        if arg == flag.long {
            return Ok(Some(((flag.env, "1".to_string()), 1)));
        }
        if let Some(short) = flag.short {
            if arg == short {
                return Ok(Some(((flag.env, "1".to_string()), 1)));
            }
        }
        if arg.starts_with(&format!("{}=", flag.long)) {
            return Err(format!("{} takes no value", flag.long));
        }
    }
    for param in PARAMS {
        let inline_value = if arg == param.long || param.short == Some(arg.as_str()) {
            None // `--flag value` form: value is the next arg
        } else if let Some(v) = arg.strip_prefix(&format!("{}=", param.long)) {
            Some(v.to_string()) // `--flag=value`
        } else if let Some(short) = param.short {
            arg.strip_prefix(short)
                .filter(|rest| !rest.is_empty() && !rest.starts_with('-'))
                .map(|rest| rest.strip_prefix('=').unwrap_or(rest).to_string()) // `-svalue`/`-s=value`
        } else {
            None
        };
        match inline_value {
            Some(value) => {
                return Ok(Some((validated(param, &value)?, 1)));
            }
            None if arg == param.long || param.short == Some(arg.as_str()) => {
                let value = args
                    .get(start + 1)
                    .ok_or_else(|| format!("{} requires a value", param.long))?;
                return Ok(Some((validated(param, value)?, 2)));
            }
            None => continue,
        }
    }
    Ok(None)
}

/// Validates `value` for `param` and returns its canonical env assignment.
fn validated(param: &Param, value: &str) -> Result<(&'static str, String), String> {
    // Run the same parsing the child would, so bad values fail in the wrapper.
    apply_value(&mut Config::default(), param.env, value)?;
    if param.env == "BENCH_CRATES" {
        // Canonical comma-separated form (and deduped/canonical order).
        let names = parse_crates(value)?
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(",");
        return Ok((param.env, names));
    }
    Ok((param.env, value.to_string()))
}

/// Build one env assignment list per crate for `--isolate-crates` mode: each
/// child process measures exactly one crate with otherwise identical workload
/// env, so heap/code/frequency warmup is paid equally by every crate.
pub fn isolate_envs(
    base: &[(&'static str, String)],
    crate_names: &[String],
) -> Vec<Vec<(&'static str, String)>> {
    crate_names
        .iter()
        .map(|name| {
            let mut child: Vec<(&'static str, String)> = base
                .iter()
                .filter(|(k, _)| *k != "BENCH_CRATES")
                .cloned()
                .collect();
            child.push(("BENCH_CRATES", name.clone()));
            child
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Env(Vec<(&'static str, &'static str)>);

    impl Env {
        fn lookup(&self, name: &str) -> Option<String> {
            self.0
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    fn from_env_pairs(pairs: &[(&'static str, &'static str)]) -> Result<Config, String> {
        let env = Env(pairs.to_vec());
        Config::from_lookup(|name| env.lookup(name))
    }

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    // --- from_lookup / env configuration ---

    #[test]
    fn defaults_when_no_env_set() {
        let cfg = from_env_pairs(&[]).unwrap();
        assert_eq!(cfg, Config::default());
        assert_eq!(cfg.message_size, 256);
        assert_eq!(cfg.messages_no, 100_000);
        assert_eq!(cfg.senders, 1_000);
        assert_eq!(cfg.receivers, 10);
        assert_eq!(cfg.latency_rounds, 100_000);
        assert_eq!(cfg.latency_warmup, 1_000);
        assert_eq!(cfg.cooldown_secs, 0);
    }

    #[test]
    fn env_overrides_every_field() {
        let cfg = from_env_pairs(&[
            ("BENCH_MESSAGE_SIZE", "32"),
            ("BENCH_MSG_NO", "5000"),
            ("BENCH_SENDERS", "8"),
            ("BENCH_RECEIVERS", "2"),
            ("BENCH_METRIC", "latency"),
            ("BENCH_CRATES", "loole tokio"),
            ("BENCH_LATENCY_ROUNDS", "777"),
            ("BENCH_LATENCY_WARMUP", "10"),
            ("BENCH_COOLDOWN_SECS", "90"),
        ])
        .unwrap();
        assert_eq!(cfg.message_size, 32);
        assert_eq!(cfg.messages_no, 5_000);
        assert_eq!(cfg.senders, 8);
        assert_eq!(cfg.receivers, 2);
        assert_eq!(cfg.metric, Metric::Latency);
        assert_eq!(cfg.crates, vec![CrateName::Loole, CrateName::Tokio]);
        assert_eq!(cfg.latency_rounds, 777);
        assert_eq!(cfg.latency_warmup, 10);
        assert_eq!(cfg.cooldown_secs, 90);
    }

    #[test]
    fn cooldown_defaults_to_zero_and_accepts_zero() {
        assert_eq!(from_env_pairs(&[]).unwrap().cooldown_secs, 0);
        assert_eq!(
            from_env_pairs(&[("BENCH_COOLDOWN_SECS", "0")])
                .unwrap()
                .cooldown_secs,
            0
        );
    }

    #[test]
    fn cooldown_rejects_non_numbers_and_negatives() {
        assert!(from_env_pairs(&[("BENCH_COOLDOWN_SECS", "abc")]).is_err());
        assert!(from_env_pairs(&[("BENCH_COOLDOWN_SECS", "-5")]).is_err());
        assert!(from_env_pairs(&[("BENCH_COOLDOWN_SECS", "")]).is_err());
    }

    #[test]
    fn invalid_number_is_error() {
        let err = from_env_pairs(&[("BENCH_MESSAGE_SIZE", "abc")]).unwrap_err();
        assert!(
            err.contains("BENCH_MESSAGE_SIZE"),
            "error names the var: {err}"
        );
    }

    #[test]
    fn zero_senders_rejected() {
        assert!(from_env_pairs(&[("BENCH_SENDERS", "0")]).is_err());
    }

    #[test]
    fn invalid_metric_is_error() {
        assert!(from_env_pairs(&[("BENCH_METRIC", "banana")]).is_err());
    }

    #[test]
    fn unknown_crate_is_error() {
        assert!(from_env_pairs(&[("BENCH_CRATES", "loole nosuch")]).is_err());
    }

    // --- Metric ---

    #[test]
    fn metric_includes() {
        assert!(Metric::Both.includes_throughput() && Metric::Both.includes_latency());
        assert!(Metric::Throughput.includes_throughput() && !Metric::Throughput.includes_latency());
        assert!(Metric::Latency.includes_latency() && !Metric::Latency.includes_throughput());
    }

    // --- parse_crates (ported from the deleted runner's main.rs) ---

    #[test]
    fn parse_crates_all_and_empty_select_everything() {
        assert_eq!(parse_crates("all").unwrap(), ALL_CRATES.to_vec());
        assert_eq!(parse_crates("").unwrap(), ALL_CRATES.to_vec());
        assert_eq!(parse_crates("   ").unwrap(), ALL_CRATES.to_vec());
    }

    #[test]
    fn parse_crates_reorders_into_canonical_order() {
        let got = parse_crates("crossbeam-channel tokio flume").unwrap();
        assert_eq!(
            got,
            vec![
                CrateName::Flume,
                CrateName::Tokio,
                CrateName::CrossbeamChannel
            ]
        );
    }

    #[test]
    fn parse_crates_deduplicates() {
        let got = parse_crates("flume flume,flume tokio").unwrap();
        assert_eq!(got, vec![CrateName::Flume, CrateName::Tokio]);
    }

    #[test]
    fn parse_crates_accepts_kanal_alias_and_canonical_name() {
        assert_eq!(parse_crates("kanal").unwrap(), vec![CrateName::Kanal]);
        assert_eq!(
            parse_crates("kanal (std-mutex)").unwrap(),
            vec![CrateName::Kanal]
        );
    }

    #[test]
    fn parse_crates_rejects_unknown_name() {
        assert!(parse_crates("loole nosuch").is_err());
    }

    // --- split_args ---

    #[test]
    fn no_custom_flags_passes_everything_through() {
        let (env, rest) = split_args(&s(&["--list", "--sample-size", "20", "spsc"])).unwrap();
        assert!(env.is_empty());
        assert_eq!(rest, s(&["--list", "--sample-size", "20", "spsc"]));
    }

    #[test]
    fn long_flag_with_separate_value() {
        let (env, rest) = split_args(&s(&["--message-size", "32"])).unwrap();
        assert_eq!(env, vec![("BENCH_MESSAGE_SIZE", "32".to_string())]);
        assert!(rest.is_empty());
    }

    #[test]
    fn long_flag_with_equals_value() {
        let (env, rest) = split_args(&s(&["--message-size=32"])).unwrap();
        assert_eq!(env, vec![("BENCH_MESSAGE_SIZE", "32".to_string())]);
        assert!(rest.is_empty());
    }

    #[test]
    fn short_flags_with_space_and_joined() {
        let (env, _) = split_args(&s(&["-m", "32"])).unwrap();
        assert_eq!(env, vec![("BENCH_MESSAGE_SIZE", "32".to_string())]);
        let (env, _) = split_args(&s(&["-m32"])).unwrap();
        assert_eq!(env, vec![("BENCH_MESSAGE_SIZE", "32".to_string())]);
        let (env, _) = split_args(&s(&["-n", "5000"])).unwrap();
        assert_eq!(env, vec![("BENCH_MSG_NO", "5000".to_string())]);
    }

    #[test]
    fn mixed_custom_and_criterion_args_keep_order() {
        let (env, rest) = split_args(&s(&[
            "--max-senders",
            "8",
            "--sample-size",
            "20",
            "--metric",
            "latency",
            "--list",
        ]))
        .unwrap();
        assert_eq!(
            env,
            vec![
                ("BENCH_SENDERS", "8".to_string()),
                ("BENCH_METRIC", "latency".to_string())
            ]
        );
        assert_eq!(rest, s(&["--sample-size", "20", "--list"]));
    }

    #[test]
    fn crates_env_value_is_canonical_comma_list() {
        let (env, _) = split_args(&s(&["--crates", "tokio loole,loole"])).unwrap();
        assert_eq!(env, vec![("BENCH_CRATES", "loole,tokio".to_string())]);
        // kanal alias resolves to the canonical display name
        let (env, _) = split_args(&s(&["--crates", "kanal"])).unwrap();
        assert_eq!(env, vec![("BENCH_CRATES", "kanal (std-mutex)".to_string())]);
    }

    #[test]
    fn missing_value_at_end_is_error() {
        assert!(split_args(&s(&["--message-size"])).is_err());
        assert!(split_args(&s(&["-m"])).is_err());
    }

    #[test]
    fn invalid_values_are_errors() {
        assert!(split_args(&s(&["--message-size", "abc"])).is_err());
        assert!(split_args(&s(&["--messages-no", "0"])).is_err());
        assert!(split_args(&s(&["--metric", "banana"])).is_err());
        assert!(split_args(&s(&["--crates", "nosuch"])).is_err());
    }

    #[test]
    fn no_charts_flag_and_env() {
        let (env, rest) = split_args(&s(&["--no-charts"])).unwrap();
        assert_eq!(env, vec![("BENCH_NO_CHARTS", "1".to_string())]);
        assert!(rest.is_empty());
        assert!(split_args(&s(&["--no-charts=1"])).is_err());
        assert!(split_args(&s(&["--no-charts", "value"])).is_ok()); // next arg stays passthrough

        assert!(
            from_env_pairs(&[("BENCH_NO_CHARTS", "1")])
                .unwrap()
                .no_charts
        );
        assert!(
            from_env_pairs(&[("BENCH_NO_CHARTS", "true")])
                .unwrap()
                .no_charts
        );
        assert!(
            !from_env_pairs(&[("BENCH_NO_CHARTS", "0")])
                .unwrap()
                .no_charts
        );
        assert!(
            !from_env_pairs(&[("BENCH_NO_CHARTS", "off")])
                .unwrap()
                .no_charts
        );
        assert!(from_env_pairs(&[("BENCH_NO_CHARTS", "banana")]).is_err());
        assert!(!from_env_pairs(&[]).unwrap().no_charts);
    }

    #[test]
    fn sampling_overrides_detect_both_forms() {
        let both = sampling_overrides_from(&s(&["--sample-size", "40"]));
        assert!(both.sample_size && !both.warm_up_time && !both.measurement_time);

        let eq = sampling_overrides_from(&s(&["--sample-size=40", "--measurement-time=10"]));
        assert!(eq.sample_size && eq.measurement_time && !eq.warm_up_time);

        assert_eq!(
            sampling_overrides_from(&s(&["--list"])),
            SamplingOverrides::default()
        );

        // Similar-but-different flags must not match.
        assert_eq!(
            sampling_overrides_from(&s(&["--sample", "40", "--warm-up-times"])),
            SamplingOverrides::default()
        );
    }

    #[test]
    fn cli_value_round_trips_through_env() {
        // The wrapper writes env assignments the child's from_env must re-read.
        let (env, _) = split_args(&s(&["-m", "32", "--crates", "tokio"])).unwrap();
        let cfg = Config::from_lookup(|name| {
            env.iter().find(|(k, _)| *k == name).map(|(_, v)| v.clone())
        })
        .unwrap();
        assert_eq!(cfg.message_size, 32);
        assert_eq!(cfg.crates, vec![CrateName::Tokio]);
    }

    #[test]
    fn new_options_defaults() {
        let cfg = from_env_pairs(&[]).unwrap();
        assert_eq!(cfg.worker_threads, 0);
        assert!(!cfg.isolate_crates);
        assert!(!cfg.no_warmup_cell);
        assert_eq!(cfg.payload, crate::message_type::PayloadKind::Alloc);
    }

    #[test]
    fn new_options_parse() {
        let cfg = from_env_pairs(&[
            ("BENCH_WORKER_THREADS", "4"),
            ("BENCH_ISOLATE_CRATES", "true"),
            ("BENCH_NO_WARMUP_CELL", "1"),
            ("BENCH_PAYLOAD", "shared"),
        ])
        .unwrap();
        assert_eq!(cfg.worker_threads, 4);
        assert!(cfg.isolate_crates);
        assert!(cfg.no_warmup_cell);
        assert_eq!(cfg.payload, crate::message_type::PayloadKind::Shared);
    }

    #[test]
    fn new_options_reject_garbage() {
        assert!(from_env_pairs(&[("BENCH_WORKER_THREADS", "abc")]).is_err());
        assert!(from_env_pairs(&[("BENCH_PAYLOAD", "banana")]).is_err());
        assert!(from_env_pairs(&[("BENCH_ISOLATE_CRATES", "maybe")]).is_err());
    }

    #[test]
    fn split_args_routes_new_flags_to_env() {
        let (env, rest) = split_args(&s(&[
            "--worker-threads",
            "4",
            "--isolate-crates",
            "--payload=shared",
            "spsc",
        ]))
        .unwrap();
        assert_eq!(
            env,
            vec![
                ("BENCH_WORKER_THREADS", "4".to_string()),
                ("BENCH_ISOLATE_CRATES", "1".to_string()),
                ("BENCH_PAYLOAD", "shared".to_string()),
            ]
        );
        assert_eq!(rest, s(&["spsc"]));
        let (env, _) = split_args(&s(&["--no-warmup-cell"])).unwrap();
        assert_eq!(env, vec![("BENCH_NO_WARMUP_CELL", "1".to_string())]);
    }

    #[test]
    fn isolate_envs_splits_one_crate_per_process() {
        let base = vec![("BENCH_MSG_NO", "20000".to_string())];
        let out = isolate_envs(&base, &["loole".to_string(), "flume".to_string()]);
        assert_eq!(out.len(), 2);
        assert!(out[0].contains(&("BENCH_CRATES", "loole".to_string())));
        assert!(out[1].contains(&("BENCH_CRATES", "flume".to_string())));
        // Shared workload env is preserved in every child.
        for child in &out {
            assert!(child.contains(&("BENCH_MSG_NO", "20000".to_string())));
        }
    }

    #[test]
    fn isolate_envs_single_crate_is_identity() {
        let base: Vec<(&'static str, String)> = vec![];
        let out = isolate_envs(&base, &["loole".to_string()]);
        assert_eq!(out, vec![vec![("BENCH_CRATES", "loole".to_string())]]);
    }
}
