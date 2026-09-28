//! Benchmark suite for loole and comparable channel crates.
//!
//! Measurement is driven by criterion (see `benches/criterion_benchmarks.rs`);
//! this library provides the per-crate adapters, the support matrix, and the
//! configuration plumbing shared by the harness.

pub mod bench_utils;
pub mod benchmark;
pub mod chart_update;
pub mod config;
pub mod ids;
pub mod machine;
pub mod message_type;
pub mod run_marker;
pub mod warmup;

mod async_channel_bench;
mod crossbeam_channel_bench;
mod flume_bench;
mod kanal_bench;
mod loole_bench;
mod tokio_bench;
