# Throughput benchmark results

Throughput (messages received per second, higher is better) measured with the
default `touched` payload: each message is a distinct allocation with every
byte filled. A separate `unbounded` chart shows the unbounded configuration.
Methodology and environment: [BENCHMARK.md](BENCHMARK.md).

### Throughput

The number of messages received per second (higher is better).

#### MPSC

![MPSC: sync-sync](benchmark/charts/images/mpsc-sync-sync-throughput-256.svg)
![MPSC: async-async](benchmark/charts/images/mpsc-async-async-throughput-256.svg)
![MPSC: async-sync](benchmark/charts/images/mpsc-async-sync-throughput-256.svg)
![MPSC: sync-async](benchmark/charts/images/mpsc-sync-async-throughput-256.svg)
![MPSC: unbounded](benchmark/charts/images/mpsc-unbounded-throughput-256.svg)

#### MPMC

![MPMC: sync-sync](benchmark/charts/images/mpmc-sync-sync-throughput-256.svg)
![MPMC: async-async](benchmark/charts/images/mpmc-async-async-throughput-256.svg)
![MPMC: async-sync](benchmark/charts/images/mpmc-async-sync-throughput-256.svg)
![MPMC: sync-async](benchmark/charts/images/mpmc-sync-async-throughput-256.svg)
![MPMC: unbounded](benchmark/charts/images/mpmc-unbounded-throughput-256.svg)

#### SPSC

![SPSC: sync-sync](benchmark/charts/images/spsc-sync-sync-throughput-256.svg)
![SPSC: async-async](benchmark/charts/images/spsc-async-async-throughput-256.svg)
![SPSC: async-sync](benchmark/charts/images/spsc-async-sync-throughput-256.svg)
![SPSC: sync-async](benchmark/charts/images/spsc-sync-async-throughput-256.svg)
![SPSC: unbounded](benchmark/charts/images/spsc-unbounded-throughput-256.svg)
