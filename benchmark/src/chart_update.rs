//! Decides whether a finished bench run should automatically update the
//! chart JSONs/SVGs.
//!
//! The converter rebuilds each `benchmark-result-*.json` purely from the
//! latest run's cells, so an unfiltered full-matrix run is the only kind that
//! may auto-update — a filtered/subset run would otherwise rewrite complete
//! charts with partial data.

use std::fmt::Display;

use crate::benchmark::ALL_CRATES;
use crate::config::Config;

/// Why the automatic chart update does not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// The bench child exited non-zero.
    BenchFailed,
    /// `--no-charts` / `BENCH_NO_CHARTS`.
    Disabled,
    /// `--list`, `--help`, or `--version` — nothing was measured.
    MetaCommand,
    /// The run covers only part of the matrix (metric filter, crate subset,
    /// or a positional criterion filter).
    PartialRun,
}

impl Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkipReason::BenchFailed => write!(f, "benchmark run failed"),
            SkipReason::Disabled => write!(f, "disabled via --no-charts"),
            SkipReason::MetaCommand => write!(f, "no benchmarks ran"),
            SkipReason::PartialRun => write!(f, "partial run"),
        }
    }
}

/// Criterion CLI flags that consume a separate value argument; everything
/// else that isn't a flag is a positional benchmark filter.
const VALUE_FLAGS: &[&str] = &[
    "--sample-size",
    "--measurement-time",
    "--warm-up-time",
    "--nresamples",
    "--noise-threshold",
    "--confidence-level",
    "--significance-level",
    "--profile-time",
    "--baseline",
    "--save-baseline",
    "--load-baseline",
];

/// Flags that mean the invocation produces no measurements.
const META_FLAGS: &[&str] = &["--list", "--help", "-h", "--version"];

/// True when the invocation only lists/prints and measures nothing
/// (`--list`/`--help`/`--version`). Such runs must not touch the run markers:
/// they write no estimates, so a marker pointing at them would make the chart
/// converter find no fresh cells.
pub fn is_meta_run(passthrough: &[String]) -> bool {
    passthrough.iter().any(|a| META_FLAGS.contains(&a.as_str()))
}

/// True when a bare positional token (a criterion benchmark filter) appears.
fn has_positional_filter(passthrough: &[String]) -> bool {
    let mut iter = passthrough.iter();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            return iter.next().is_some();
        }
        if arg.starts_with('-') {
            // Consume the value of flags written in `--flag value` form;
            // `--flag=value` is a single token and needs no skipping.
            if VALUE_FLAGS.contains(&arg.as_str()) {
                iter.next();
            }
        } else {
            return true;
        }
    }
    false
}

/// Decides whether to regenerate charts after the bench child exits.
pub fn should_update_charts(
    cfg: &Config,
    passthrough: &[String],
    bench_ok: bool,
) -> Result<(), SkipReason> {
    if !bench_ok {
        return Err(SkipReason::BenchFailed);
    }
    if cfg.no_charts {
        return Err(SkipReason::Disabled);
    }
    if is_meta_run(passthrough) {
        return Err(SkipReason::MetaCommand);
    }
    if !cfg.metric.includes_throughput()
        || !cfg.metric.includes_latency()
        || cfg.crates != ALL_CRATES
        || has_positional_filter(passthrough)
    {
        return Err(SkipReason::PartialRun);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark::CrateName;
    use crate::config::Metric;

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn meta_run_detection() {
        assert!(is_meta_run(&s(&["--list"])));
        assert!(is_meta_run(&s(&["--bench", "--list"])));
        assert!(!is_meta_run(&s(&["--bench"])));
        assert!(!is_meta_run(&s(&["--sample-size", "40"])));
    }

    #[test]
    fn full_run_updates_charts() {
        assert_eq!(should_update_charts(&Config::default(), &[], true), Ok(()));
        // Sampling-depth tweaks do not change coverage.
        assert_eq!(
            should_update_charts(
                &Config::default(),
                &s(&["--bench", "--sample-size", "40", "--save-baseline=x"]),
                true
            ),
            Ok(())
        );
    }

    #[test]
    fn failed_bench_never_updates_charts() {
        assert_eq!(
            should_update_charts(&Config::default(), &[], false),
            Err(SkipReason::BenchFailed)
        );
    }

    #[test]
    fn no_charts_flag_skips() {
        let cfg = Config {
            no_charts: true,
            ..Config::default()
        };
        assert_eq!(
            should_update_charts(&cfg, &[], true),
            Err(SkipReason::Disabled)
        );
    }

    #[test]
    fn meta_commands_skip() {
        for args in [
            &["--list"][..],
            &["--help"][..],
            &["-h"][..],
            &["--version"][..],
        ] {
            assert_eq!(
                should_update_charts(&Config::default(), &s(args), true),
                Err(SkipReason::MetaCommand),
                "{args:?}"
            );
        }
    }

    #[test]
    fn metric_subset_is_partial() {
        for metric in [Metric::Throughput, Metric::Latency] {
            let cfg = Config {
                metric,
                ..Config::default()
            };
            assert_eq!(
                should_update_charts(&cfg, &[], true),
                Err(SkipReason::PartialRun)
            );
        }
    }

    #[test]
    fn crate_subset_is_partial() {
        let cfg = Config {
            crates: vec![CrateName::Loole],
            ..Config::default()
        };
        assert_eq!(
            should_update_charts(&cfg, &[], true),
            Err(SkipReason::PartialRun)
        );
    }

    #[test]
    fn positional_filter_is_partial() {
        assert_eq!(
            should_update_charts(&Config::default(), &s(&["spsc"]), true),
            Err(SkipReason::PartialRun)
        );
        // Value-taking flags must not have their values mistaken for filters.
        assert_eq!(
            should_update_charts(
                &Config::default(),
                &s(&["--sample-size", "30", "spsc"]),
                true
            ),
            Err(SkipReason::PartialRun)
        );
        // Same, but without the trailing filter: full run.
        assert_eq!(
            should_update_charts(&Config::default(), &s(&["--sample-size", "30"]), true),
            Ok(())
        );
        // `--` switches to filter positionals.
        assert_eq!(
            should_update_charts(&Config::default(), &s(&["--", "loole"]), true),
            Err(SkipReason::PartialRun)
        );
        // Inline `--flag=value` forms are single tokens.
        assert_eq!(
            should_update_charts(
                &Config::default(),
                &s(&["--measurement-time=10", "--warm-up-time=1"]),
                true
            ),
            Ok(())
        );
    }
}
