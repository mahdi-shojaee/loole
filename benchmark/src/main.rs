#![allow(dead_code, unused_imports, unused_variables)]

use std::io::prelude::*;
use std::io::prelude::*;
use std::path::Path;
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

    match std::process::Command::new("npm")
        .args(["install"])
        .current_dir(&charts_dir)
        .status()
    {
        Ok(_) => {}
        Err(err) => {
            eprintln!("{}", err);
            eprintln!("Maybe node.js is not installed on your system. You can install it from https://nodejs.org/en.");
            std::process::exit(1);
        }
    }

    match std::process::Command::new("npm")
        .args(["run", "update"])
        .current_dir(&charts_dir)
        .status()
    {
        Ok(_) => println!("Charts updated successfully"),
        Err(err) => eprintln!("{}", err),
    }
}
