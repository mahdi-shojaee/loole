# Latency benchmark results

Ping-pong latency plus per-message queueing-delay distributions under load.
Methodology: [BENCHMARK.md](BENCHMARK.md).

### Latency

Latency is the round-trip time in nanoseconds (lower is better) for a single sender/receiver pair exchanging messages back and forth across two channels (one forward, one back).

#### SPSC

![SPSC: sync-sync latency](benchmark/charts/images/spsc-sync-sync-latency-256.svg)
![SPSC: async-async latency](benchmark/charts/images/spsc-async-async-latency-256.svg)
![SPSC: async-sync latency](benchmark/charts/images/spsc-async-sync-latency-256.svg)
![SPSC: sync-async latency](benchmark/charts/images/spsc-sync-async-latency-256.svg)
![SPSC: unbounded latency](benchmark/charts/images/spsc-unbounded-latency-256.svg)

## Per-message latency distribution

Each throughput run records a `target/criterion/runs/latency-dist-*.json`
sidecar with percentile tables (p10/p50/p95/p99/p99.9) per cell; the charts
below render from the newest full-run sidecar (see [Sampling](BENCHMARK.md#sampling)).
They populate on the next full run with the current harness.
