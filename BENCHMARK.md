# Benchmarking

Loole's benchmark suite compares loole against flume, async-channel, tokio,
kanal (std-mutex), and crossbeam-channel, measuring two metrics:

* **Throughput** — messages received per second (higher is better).
* **Latency** — round-trip time for a single sender/receiver pair exchanging
  messages back and forth, in nanoseconds per round trip (lower is better).

Measurements are driven by [Criterion.rs](https://github.com/bheisler/criterion.rs):
warm-up, statistical sampling, outlier detection, confidence intervals, and
run-to-run regression reports (`target/criterion/report/`) come built in.

## Default workload

| Dimension | Values |
|-----------|--------|
| Message size | 256 bytes |
| Throughput topologies | SPSC (1 sender / 1 receiver), MPSC (1000 / 1), MPMC (1000 / 10) |
| Latency topology | SPSC ping-pong (1 / 1): 100,000 timed round trips + 1,000 warm-up per iteration |
| Sender/receiver API combos | sync-sync, async-async, async-sync, sync-async |
| Channel capacities | 0, 1, 10, 50, 100, unbounded |
| Crates | loole, flume, async-channel, tokio, kanal (std-mutex), crossbeam-channel |

Throughput transfers 100,000 messages per iteration. Cells a crate cannot
support (e.g. crossbeam's async combos, tokio's MPMC) are skipped and render
as empty spots in the charts. Every dimension above is configurable — see
[Options](#options).

## How it works

1. The bench binary measures the full matrix and criterion writes per-cell
   statistics to `target/criterion/**/estimates.json`.
2. At the start of each run the suite records a marker,
    `target/criterion/runs/last-<size>-<payload>.json`, holding the run's start time. The
   chart converter only trusts estimates written after that marker, so charts
   always reflect the latest run of a given message size and
   `target/criterion` never needs manual cleaning. Meta invocations
   (`--list`, `--help`, `--version`) measure nothing and leave the marker
   alone.
3. After a **full** run — all crates, both metrics, no bench filter — the
   suite automatically rebuilds the chart set for that run's payload mode:
   `benchmark/charts/benchmark-result-{throughput,latency}-<size>.json` for
   `alloc` (historical payload-less names), with a `-touched` suffix for
   `touched`, and re-renders the SVGs. Each mode refreshes only its own set;
   the two never mix. Partial or failed runs never touch the charts;
   the suite prints the manual command instead. `shared` has no chart set.

## Quick start

```bash
# Full suite, default workload; charts regenerate automatically at the end.
# Requires Node.js only (npm install runs by itself on the first chart update).
cargo bench -p benchmark --bench criterion_benchmarks
```

A full default run takes roughly 30–40 minutes; use the options below to run
smaller subsets.

## Options

Suite options (each is also settable via its environment variable):

| Option | Short | Env var | Default | Description |
|--------|-------|---------|---------|-------------|
| `--message-size` | `-m` | `BENCH_MESSAGE_SIZE` | `256` | Message/payload size in bytes |
| `--messages-no` | `-n` | `BENCH_MSG_NO` | `100000` | Messages transferred per throughput iteration |
| `--max-senders` | | `BENCH_SENDERS` | `1000` | Sender count for MPSC (receivers = 1) and MPMC |
| `--max-receivers` | | `BENCH_RECEIVERS` | `10` | Receiver count for MPMC |
| `--metric` | | `BENCH_METRIC` | `both` | `throughput`, `latency`, or `both` |
| `--crates` | | `BENCH_CRATES` | `all` | Space/comma-separated crates (e.g. `"loole flume"`); order is execution order (e.g. `--crates "flume loole"` measures flume first) |
| `--latency-rounds` | | `BENCH_LATENCY_ROUNDS` | `100000` | Timed ping-pong round trips per iteration |
| `--latency-warmup` | | `BENCH_LATENCY_WARMUP` | `1000` | Untimed warm-up round trips per iteration |
| `--cooldown-secs` | | `BENCH_COOLDOWN_SECS` | `0` | Idle seconds between benchmark groups so the machine cools down (0 disables) |
| `--no-charts` | | `BENCH_NO_CHARTS` | off | Skip the automatic chart update after a full run |
| `--worker-threads` | | `BENCH_WORKER_THREADS` | `0` | tokio worker count (0 = one per CPU) |
| `--isolate-crates` | | `BENCH_ISOLATE_CRATES` | off | one process per crate |
| `--no-warmup-cell` | | `BENCH_NO_WARMUP_CELL` | off | skip pre-timing warmup |
| `--payload` | | `BENCH_PAYLOAD` | `alloc` | `alloc` (fresh Vec per send, allocator-inclusive), `shared` (one Arc per sender: channel-isolation control, not a workload model, never charted), or `touched` (a distinct fresh Vec per send, every byte filled with a deterministic pattern — allocator-inclusive and cache-realistic; charted as a parallel `-touched` set). Throughput cells only; the latency suite always runs `alloc`, so there are no touched latency charts. |

All native criterion options pass straight through:

| Criterion option | Suite default | Description |
|------------------|---------------|-------------|
| `--sample-size <N>` | `10` | Samples per benchmark |
| `--warm-up-time <S>` | `0.5` | Warm-up time per benchmark, in seconds |
| `--measurement-time <S>` | `1.5` | Sampling time budget per benchmark, in seconds |
| `--save-baseline <name>` / `--baseline <name>` | | Store / compare against a named baseline |
| `--list` | | List benchmarks without running them |
| `<filter>` | | Positional substring filter, e.g. `spsc` or `spsc/sync-sync` |

The in-code sampling defaults are quick so a full ~450-cell run stays
practical; any sampling flag set on the command line overrides them. Deepen
sampling for numbers you intend to publish.

## Examples

```bash
# Everything, default workload
cargo bench -p benchmark --bench criterion_benchmarks

# 1024-byte messages, fewer senders/receivers
cargo bench -p benchmark --bench criterion_benchmarks -- -m 1024 --max-senders 500 --max-receivers 5

# Quick subset via criterion's filter (only spsc cells, loole + flume only)
cargo bench -p benchmark --bench criterion_benchmarks -- --crates "loole flume" spsc

# Latency only
cargo bench -p benchmark --bench criterion_benchmarks -- --metric latency

# Fast smoke run (tiny workload, short sampling)
cargo bench -p benchmark --bench criterion_benchmarks -- \
  -n 1000 --latency-rounds 1000 --max-senders 16 --max-receivers 4 \
  --warm-up-time 0.1 --measurement-time 0.2

# Publication-grade sampling (slower, tighter confidence intervals)
cargo bench -p benchmark --bench criterion_benchmarks -- --sample-size 40 --measurement-time 10
```

## Charts

Full bench runs update the charts automatically; the command below is the
manual form — useful after a partial (filtered) run:

```bash
cd benchmark/charts
npm install                 # only needed when node_modules is missing
npm run update -- <size> [payload]  # <size> must match the measured message size; payload is alloc (default) or touched
```

The converter rebuilds each chart cell from the newest estimates of the
latest run of that message size, keeping the deepest workload per cell;
never-measured cells (unsupported combos) are zero and render as no bar. The
converter has its own test suite: `npm test`.

## Regression tracking

Criterion compares every run against the previous one out of the box; to
measure the effect of a code change, pin a named baseline before the change
and compare against it after:

```bash
# 1. Baseline run, on the old code
cargo bench -p benchmark --bench criterion_benchmarks -- --save-baseline main

# 2. Apply the change …

# 3. Same workload again, compared against the stored baseline
cargo bench -p benchmark --bench criterion_benchmarks -- --baseline main
```

Each cell reports its change (faster/slower, with the p-value and confidence
interval) at the end of the run; the full plots live under
`target/criterion/report/`. Notes:

* Run both commands with identical workload flags — baselines are keyed by
  benchmark ID, so a different message size, message count, or sender count
  produces different IDs and nothing to compare against.
* A baseline comparison is still a normal measuring run: if it covers the
  full matrix, the charts update as usual.
* Baselines live under `target/criterion/` and are not committed.
* For decisions that matter, pair baselines with deeper sampling
  (`--sample-size 40 --measurement-time 10`) so the reported deltas carry
  tighter confidence intervals.

## Machine descriptor

Every invocation writes `target/criterion/runs/machine-<size>-<payload>-<unix_ms>-<pid>.json`
(governor, per-CPU frequencies, topology, process affinity, load, memory, git
SHA + dirty flag, full invocation and `BENCH_*` env). Attach it to any
published number; check it first whenever a ranking flips between runs.
