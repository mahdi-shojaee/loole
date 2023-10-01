# Loole

A safe async/sync multi-producer multi-consumer channel.

![Github Actions](https://github.com/mahdi-shojaee/loole/actions/workflows/rust.yml/badge.svg?branch=master)
[![Documentation](https://docs.rs/loole/badge.svg)](https://docs.rs/loole)
[![Cargo](https://img.shields.io/crates/v/loole.svg)](https://crates.io/crates/loole)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](https://github.com/mahdi-shojaee/loole)

Producers can send and consumers can receive messages asynchronously or synchronously:

- async send -> async receive
- sync send -> sync receive
- async send -> sync receive
- sync send -> async receive

## Examples

```rust
let (tx, rx) = loole::unbounded();
tx.send(10).unwrap();
assert_eq!(rx.recv().unwrap(), 10);
```

## Usage

To use Loole, place the following line under the `[dependencies]` section in your `Cargo.toml`:

```
loole = "x.y"
```

## Benchmarks

### MPSC

![MPSC](misc/loole-mpsc.png)

### MPMC

![MPMC](misc/loole-mpmc.png)

### SPSC

![SPSC](misc/loole-spsc.png)

## License

Loole is licensed under either of:

- Apache License 2.0, (http://www.apache.org/licenses/LICENSE-2.0)

- MIT license (http://opensource.org/licenses/MIT)
