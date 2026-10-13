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

Per-message queueing delay under full load, unbounded cells only (where tails
grow unmasked). Rows are percentiles (ns, lower is better); bars are crates.
Each throughput run records a `target/criterion/runs/latency-dist-*.json`
sidecar with these tables; see [Sampling](BENCHMARK.md#sampling).

#### SPSC

![SPSC: sync-sync distribution](benchmark/charts/images/spsc-sync-sync-latency-dist-256.svg)
![SPSC: async-async distribution](benchmark/charts/images/spsc-async-async-latency-dist-256.svg)
![SPSC: async-sync distribution](benchmark/charts/images/spsc-async-sync-latency-dist-256.svg)
![SPSC: sync-async distribution](benchmark/charts/images/spsc-sync-async-latency-dist-256.svg)

#### MPSC

![MPSC: sync-sync distribution](benchmark/charts/images/mpsc-sync-sync-latency-dist-256.svg)
![MPSC: async-async distribution](benchmark/charts/images/mpsc-async-async-latency-dist-256.svg)
![MPSC: async-sync distribution](benchmark/charts/images/mpsc-async-sync-latency-dist-256.svg)
![MPSC: sync-async distribution](benchmark/charts/images/mpsc-sync-async-latency-dist-256.svg)

#### MPMC

![MPMC: sync-sync distribution](benchmark/charts/images/mpmc-sync-sync-latency-dist-256.svg)
![MPMC: async-async distribution](benchmark/charts/images/mpmc-async-async-latency-dist-256.svg)
![MPMC: async-sync distribution](benchmark/charts/images/mpmc-async-sync-latency-dist-256.svg)
![MPMC: sync-async distribution](benchmark/charts/images/mpmc-sync-async-latency-dist-256.svg)
