#![allow(dead_code, unused_imports, unused_variables)]

const MESSAGES_NO: usize = 1_000_000;
const MESSAGE_SIZE: usize = 256;
const MAX_SENDERS: usize = 2000;
const MAX_RECEIVERS: usize = 10;

use std::io::prelude::*;
use std::path::Path;
use std::process::Stdio;
use std::{fs::File, path::PathBuf};

use chart_data::{get_bench_data, Item};

mod bench_utils;
mod chart_data;
mod message_type;

mod async_channel_bench;
mod crossbeam_channel_bench;
mod flume_bench;
mod kanal_bench;
mod loole_bench;
mod tokio_bench;

mod benchmark;

fn run_npm_command(charts_dir: &Path, args: &[&str]) -> Result<(), String> {
    match std::process::Command::new("npm")
        .args(args)
        .current_dir(charts_dir)
        .stdout(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("npm command failed with status: {}", status)),
        Err(err) => Err(err.to_string()),
    }
}

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let bench_data = get_bench_data(&rt);
    let json_str = serde_json::to_string_pretty(&bench_data).unwrap();

    let charts_dir = match std::env::current_dir().unwrap().ends_with("charts") {
        true => PathBuf::from("."),
        false => PathBuf::from("benchmark").join("charts"),
    };

    let result_path = charts_dir.join("benchmark-result.json");

    let mut file = File::create(result_path).unwrap();
    file.write_all(json_str.as_ref()).unwrap();

    if let Err(err) = run_npm_command(&charts_dir, &["install"]) {
        eprintln!("Failed to run 'npm install': {}", err);
        eprintln!("Maybe node.js is not installed on your system. You can install it from https://nodejs.org/en.");
        std::process::exit(1);
    }

    match run_npm_command(&charts_dir, &["run", "update"]) {
        Ok(_) => println!("Charts updated successfully"),
        Err(err) => eprintln!("Failed to run 'npm run update': {}", err),
    }
}
